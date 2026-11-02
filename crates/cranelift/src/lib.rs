//! The Cranelift tier (D3, D36): compiles loops that get hot in tier 0 to machine code.
//!
//! A loop is compiled straight from tier 0's bytecode, from its head to its jump back, and works on tier 0's own
//! register files, so moving between the tiers needs no conversion: tier 0 calls the compiled loop at the head
//! and goes on from wherever it returns. Ints and booleans live in machine registers while the loop runs;
//! rationals and table reads go through the helpers in [`calls`]. Whatever a compiled loop can't finish, such as
//! an instruction about to fail, it returns to tier 0 at that instruction, before changing anything, so tier 0
//! reports exactly the error it always would (rule 10).

mod calls;

use std::collections::{BTreeSet, HashMap};

use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, Block, BlockArg, FuncRef, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind,
    Value, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use easium_bytecode::op::{Op, Reg};
use easium_bytecode::{Code, CompiledLoop, LoopCompiler};
use easium_front::ir::{Comparison, Counting};
use easium_runtime::Rational;

/// Compiles hot loops with Cranelift. It owns the machine code, so it must outlive every run it serves.
pub struct Cranelift {
    module: JITModule,
    context: Context,
    builder_context: FunctionBuilderContext,
    helpers: Helpers,
    loops: usize,
    /// How to read a table item in place, when `Value`'s layout checked out.
    layout: Option<calls::ValueLayout>,
}

/// The helpers compiled loops call, as declared in the module.
struct Helpers {
    rational_copy: FuncId,
    rational_move: FuncId,
    rational_from_int: FuncId,
    rational_divide_ints: FuncId,
    rational_arithmetic: FuncId,
    rational_compare: FuncId,
    table_length: FuncId,
    table_int: FuncId,
    table_rational: FuncId,
    table_view: FuncId,
    object_move: FuncId,
}

impl Cranelift {
    /// A compiler for this machine, or `None` if Cranelift doesn't support it.
    #[must_use]
    pub fn new() -> Option<Self> {
        let mut flags = settings::builder();
        flags.set("opt_level", "speed").ok()?;
        let isa = cranelift_native::builder()
            .ok()?
            .finish(settings::Flags::new(flags))
            .ok()?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let symbols: [(&str, *const u8); 11] = [
            ("easium_rational_copy", calls::rational_copy as *const u8),
            ("easium_rational_move", calls::rational_move as *const u8),
            (
                "easium_rational_from_int",
                calls::rational_from_int as *const u8,
            ),
            (
                "easium_rational_divide_ints",
                calls::rational_divide_ints as *const u8,
            ),
            (
                "easium_rational_arithmetic",
                calls::rational_arithmetic as *const u8,
            ),
            (
                "easium_rational_compare",
                calls::rational_compare as *const u8,
            ),
            ("easium_table_length", calls::table_length as *const u8),
            ("easium_table_int", calls::table_int as *const u8),
            ("easium_table_rational", calls::table_rational as *const u8),
            ("easium_table_view", calls::table_view as *const u8),
            ("easium_object_move", calls::object_move as *const u8),
        ];
        for (name, pointer) in symbols {
            builder.symbol(name, pointer);
        }
        let mut module = JITModule::new(builder);
        let pointer = module.target_config().pointer_type();
        let mut declare = |name: &str, params: &[types::Type]| -> Option<FuncId> {
            let mut signature = module.make_signature();
            for param in params {
                signature.params.push(AbiParam::new(*param));
            }
            signature.returns.push(AbiParam::new(types::I64));
            module
                .declare_function(name, Linkage::Import, &signature)
                .ok()
        };
        let i64 = types::I64;
        let helpers = Helpers {
            rational_copy: declare("easium_rational_copy", &[pointer, i64, i64, pointer])?,
            rational_move: declare("easium_rational_move", &[pointer, i64, i64, i64])?,
            rational_from_int: declare("easium_rational_from_int", &[pointer, i64, i64, i64])?,
            rational_divide_ints: declare(
                "easium_rational_divide_ints",
                &[pointer, i64, i64, i64, i64],
            )?,
            rational_arithmetic: declare(
                "easium_rational_arithmetic",
                &[pointer, i64, i64, i64, i64, i64],
            )?,
            rational_compare: declare("easium_rational_compare", &[pointer, i64, i64, i64])?,
            table_length: declare("easium_table_length", &[pointer, i64, i64])?,
            table_int: declare("easium_table_int", &[pointer, i64, i64, i64, pointer])?,
            table_rational: declare(
                "easium_table_rational",
                &[pointer, i64, pointer, i64, i64, i64, i64],
            )?,
            table_view: declare("easium_table_view", &[pointer, i64, i64, pointer])?,
            object_move: declare("easium_object_move", &[pointer, i64, i64, i64])?,
        };
        let context = module.make_context();
        Some(Self {
            module,
            context,
            builder_context: FunctionBuilderContext::new(),
            helpers,
            loops: 0,
            layout: calls::value_layout(),
        })
    }
}

impl LoopCompiler for Cranelift {
    fn compile(&mut self, code: &Code, head: usize, back: usize) -> Option<Box<dyn CompiledLoop>> {
        let ops = code.ops.get(head..=back)?;
        if !ops.iter().all(|op| supported(op, code)) {
            return None;
        }
        let pointer_type = self.module.target_config().pointer_type();
        let mut signature = self.module.make_signature();
        for param in [
            pointer_type,
            pointer_type,
            types::I64,
            pointer_type,
            types::I64,
        ] {
            signature.params.push(AbiParam::new(param));
        }
        signature.returns.push(AbiParam::new(types::I64));
        let name = format!("easium_loop_{}", self.loops);
        self.loops += 1;
        let id = self
            .module
            .declare_function(&name, Linkage::Local, &signature)
            .ok()?;
        self.context.func.signature = signature;
        let frontend_config = self.module.isa().frontend_config();
        let helpers = Imported::new(&self.helpers, &mut self.module, &mut self.context.func);
        let mut builder = FunctionBuilder::new(&mut self.context.func, &mut self.builder_context);
        // The loop's rational constants, reduced once here; its code copies them instead of making them again.
        let constants: Box<[Rational]> = ops
            .iter()
            .filter_map(|op| match op {
                Op::RationalConstant { value, .. } => Some(*value),
                _ => None,
            })
            .collect();
        let built = LoopBuilder::build(
            &mut builder,
            &helpers,
            ops,
            head,
            pointer_type,
            &constants,
            self.layout,
        );
        builder.seal_all_blocks();
        builder.finalize(frontend_config);
        if built.is_none() {
            self.module.clear_context(&mut self.context);
            return None;
        }
        let defined = self.module.define_function(id, &mut self.context);
        self.module.clear_context(&mut self.context);
        defined.ok()?;
        self.module.finalize_definitions().ok()?;
        let function = self.module.get_finalized_function(id);
        Some(Box::new(calls::Compiled::new(
            function,
            constants,
            code.ints,
            code.rationals,
            code.objects,
            head,
        )))
    }
}

/// Whether a compiled loop can run an instruction itself, with every register it names inside the files.
fn supported(op: &Op, code: &Code) -> bool {
    let int = |reg: &u32| (*reg as usize) < code.ints;
    let rational = |reg: &u32| (*reg as usize) < code.rationals;
    let object = |reg: &u32| (*reg as usize) < code.objects;
    let number = |reg: &Reg| match reg {
        Reg::Int(reg) | Reg::Boolean(reg) => int(reg),
        Reg::Rational(reg) => rational(reg),
        Reg::Object(_) => false,
    };
    match op {
        Op::IntConstant { dst, .. } | Op::Increment { dst } => int(dst),
        Op::MoveInt { dst, src } | Op::Not { dst, src } => int(dst) && int(src),
        Op::Add { dst, a, b, .. }
        | Op::Subtract { dst, a, b, .. }
        | Op::Multiply { dst, a, b, .. }
        | Op::Remainder { dst, a, b, .. }
        | Op::CompareInts { dst, a, b, .. } => int(dst) && int(a) && int(b),
        Op::Jump { .. } | Op::NeverReached { .. } => true,
        Op::JumpIfFalse { condition, .. } | Op::JumpIfTrue { condition, .. } => int(condition),
        Op::JumpIfAtLeast { a, b, .. } => int(a) && int(b),
        Op::CheckCount { src, .. } => int(src),
        Op::RationalConstant { dst, .. } => rational(dst),
        Op::MoveRational { dst, src } => rational(dst) && rational(src),
        Op::IntToRational { dst, src } => rational(dst) && int(src),
        Op::DivideInts { dst, a, b, .. } => rational(dst) && int(a) && int(b),
        Op::AddRationals { dst, a, b, .. }
        | Op::SubtractRationals { dst, a, b, .. }
        | Op::MultiplyRationals { dst, a, b, .. }
        | Op::DivideRationals { dst, a, b, .. } => rational(dst) && rational(a) && rational(b),
        Op::CompareRationals { dst, a, b, .. } => int(dst) && rational(a) && rational(b),
        Op::Length { dst, table } => int(dst) && object(table),
        Op::MoveObject { dst, src } => object(dst) && object(src),
        Op::Item { dst, table, index }
        | Op::ReadPosition {
            dst, table, index, ..
        } => number(dst) && object(table) && int(index),
        _ => false,
    }
}

/// The helpers, imported into the function being built.
struct Imported {
    rational_copy: FuncRef,
    rational_move: FuncRef,
    rational_from_int: FuncRef,
    rational_divide_ints: FuncRef,
    rational_arithmetic: FuncRef,
    rational_compare: FuncRef,
    table_length: FuncRef,
    table_int: FuncRef,
    table_rational: FuncRef,
    table_view: FuncRef,
    object_move: FuncRef,
}

impl Imported {
    fn new(
        helpers: &Helpers,
        module: &mut JITModule,
        func: &mut cranelift_codegen::ir::Function,
    ) -> Self {
        let mut import = |id: FuncId| module.declare_func_in_func(id, func);
        Self {
            rational_copy: import(helpers.rational_copy),
            rational_move: import(helpers.rational_move),
            rational_from_int: import(helpers.rational_from_int),
            rational_divide_ints: import(helpers.rational_divide_ints),
            rational_arithmetic: import(helpers.rational_arithmetic),
            rational_compare: import(helpers.rational_compare),
            table_length: import(helpers.table_length),
            table_int: import(helpers.table_int),
            table_rational: import(helpers.table_rational),
            table_view: import(helpers.table_view),
            object_move: import(helpers.object_move),
        }
    }
}

/// Builds one compiled loop: a block per instruction, ints in variables, and one exit that stores them back.
struct LoopBuilder<'b, 'f> {
    builder: &'b mut FunctionBuilder<'f>,
    helpers: &'b Imported,
    head: usize,
    blocks: Vec<Block>,
    exit: Block,
    outside: HashMap<usize, Block>,
    /// Blocks that exit to an instruction, filled in after every instruction's block (a block can't be left
    /// half-built to fill another).
    pending: Vec<(Block, usize)>,
    variables: HashMap<u32, Variable>,
    ints: Value,
    rationals: Value,
    rationals_length: Value,
    objects: Value,
    objects_length: Value,
    out: Value,
    pointer_type: types::Type,
    /// Where each rational constant is, in the order the loop's instructions use them.
    constants: Vec<i64>,
    next_constant: usize,
    /// How to read a table item in place, if `Value`'s layout checked out.
    layout: Option<calls::ValueLayout>,
    /// For each table the loop reads, variables holding where its items are and how many there are.
    views: HashMap<u32, (Variable, Variable)>,
    /// A 16-byte stack slot views are fetched into.
    view_slot: Option<Value>,
}

impl<'b, 'f> LoopBuilder<'b, 'f> {
    fn build(
        builder: &'b mut FunctionBuilder<'f>,
        helpers: &'b Imported,
        ops: &[Op],
        head: usize,
        pointer_type: types::Type,
        constants: &[Rational],
        layout: Option<calls::ValueLayout>,
    ) -> Option<()> {
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let params = builder.block_params(entry).to_vec();
        let [ints, rationals, rationals_length, objects, objects_length] = params[..] else {
            return None;
        };
        let slot =
            builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let out = builder.ins().stack_addr(pointer_type, slot, 0);
        let mut used = BTreeSet::new();
        for op in ops {
            used.extend(int_registers(op));
        }
        let mut variables = HashMap::new();
        for reg in used {
            let variable = builder.declare_var(types::I64);
            let value = builder
                .ins()
                .load(types::I64, MemFlagsData::trusted(), ints, offset(reg)?);
            builder.def_var(variable, value);
            variables.insert(reg, variable);
        }
        let blocks = (0..ops.len())
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        let exit = builder.create_block();
        builder.append_block_param(exit, types::I64);
        let first = *blocks.first()?;
        let mut loop_builder = LoopBuilder {
            builder,
            helpers,
            head,
            blocks,
            exit,
            outside: HashMap::new(),
            pending: Vec::new(),
            variables,
            ints,
            rationals,
            rationals_length,
            objects,
            objects_length,
            out,
            pointer_type,
            constants: constants
                .iter()
                .map(|constant| i64::try_from(std::ptr::from_ref(constant).addr()).ok())
                .collect::<Option<_>>()?,
            next_constant: 0,
            layout,
            views: HashMap::new(),
            view_slot: None,
        };
        loop_builder.prepare_views(ops)?;
        loop_builder.builder.ins().jump(first, &[]);
        for (at, op) in ops.iter().enumerate() {
            let block = *loop_builder.blocks.get(at)?;
            loop_builder.builder.switch_to_block(block);
            loop_builder.instruction(at, op)?;
        }
        loop_builder.finish_exit()
    }

    /// At loop entry, finds where each table the loop reads keeps its items. If one isn't an in-order table after
    /// all, the loop goes back to tier 0 at its head.
    fn prepare_views(&mut self, ops: &[Op]) -> Option<()> {
        if self.layout.is_none() {
            return Some(());
        }
        let tables = ops
            .iter()
            .filter_map(|op| match op {
                Op::Length { table, .. }
                | Op::Item {
                    table,
                    dst: Reg::Int(_) | Reg::Boolean(_),
                    ..
                }
                | Op::ReadPosition {
                    table,
                    dst: Reg::Int(_) | Reg::Boolean(_),
                    ..
                } => Some(*table),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        if tables.is_empty() {
            return Some(());
        }
        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            16,
            3,
        ));
        self.view_slot = Some(self.builder.ins().stack_addr(self.pointer_type, slot, 0));
        for table in tables {
            let address_variable = self.builder.declare_var(types::I64);
            let length_variable = self.builder.declare_var(types::I64);
            self.views
                .insert(table, (address_variable, length_variable));
            self.fetch_view(table, self.head)?;
        }
        Some(())
    }

    /// Finds where a table's items are now, into its view variables; if it isn't an in-order table, hands back to
    /// tier 0 at instruction `resume`.
    fn fetch_view(&mut self, table: u32, resume: usize) -> Option<()> {
        let (address_variable, length_variable) = *self.views.get(&table)?;
        let out = self.view_slot?;
        let args = [
            self.objects,
            self.objects_length,
            self.constant(i64::from(table)),
            out,
        ];
        let failed = self.call(self.helpers.table_view, &args)?;
        self.bail_to_if(resume, failed);
        let address = self
            .builder
            .ins()
            .load(types::I64, MemFlagsData::trusted(), out, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlagsData::trusted(), out, 8);
        self.builder.def_var(address_variable, address);
        self.builder.def_var(length_variable, length);
        Some(())
    }

    /// The exit: every int variable back to tier 0's registers, then where tier 0 goes on from.
    fn finish_exit(&mut self) -> Option<()> {
        for (block, pc) in std::mem::take(&mut self.pending) {
            self.builder.switch_to_block(block);
            let resume = self
                .builder
                .ins()
                .iconst(types::I64, i64::try_from(pc).ok()?);
            self.builder
                .ins()
                .jump(self.exit, &[BlockArg::Value(resume)]);
        }
        self.builder.switch_to_block(self.exit);
        let resume = *self.builder.block_params(self.exit).first()?;
        let mut stores = self
            .variables
            .iter()
            .map(|(reg, variable)| (*reg, *variable))
            .collect::<Vec<_>>();
        stores.sort_unstable_by_key(|(reg, _)| *reg);
        for (reg, variable) in stores {
            let value = self.builder.use_var(variable);
            self.builder
                .ins()
                .store(MemFlagsData::trusted(), value, self.ints, offset(reg)?);
        }
        self.builder.ins().return_(&[resume]);
        Some(())
    }

    fn var(&mut self, reg: u32) -> Option<Value> {
        let variable = *self.variables.get(&reg)?;
        Some(self.builder.use_var(variable))
    }

    fn set(&mut self, reg: u32, value: Value) -> Option<()> {
        let variable = *self.variables.get(&reg)?;
        self.builder.def_var(variable, value);
        Some(())
    }

    /// The block for instruction `pc`: inside the loop, its own block; outside, one that exits to it.
    fn block_for(&mut self, pc: usize) -> Block {
        if let Some(block) = pc.checked_sub(self.head).and_then(|at| self.blocks.get(at)) {
            return *block;
        }
        if let Some(block) = self.outside.get(&pc) {
            return *block;
        }
        let block = self.builder.create_block();
        self.pending.push((block, pc));
        self.outside.insert(pc, block);
        block
    }

    /// After instruction `at`, on to the next.
    fn next(&mut self, at: usize) {
        let block = self.block_for(self.head + at + 1);
        self.builder.ins().jump(block, &[]);
    }

    /// Hands instruction `at` back to tier 0 when `failed` is non-zero; otherwise goes on in a new block.
    fn bail_if(&mut self, at: usize, failed: Value) {
        self.bail_to_if(self.head + at, failed);
    }

    /// Goes back to tier 0 at instruction `resume` when `failed` is non-zero; otherwise goes on in a new block.
    fn bail_to_if(&mut self, resume: usize, failed: Value) {
        let bail = self.block_for_bail(resume - self.head);
        let go_on = self.builder.create_block();
        self.builder.ins().brif(failed, bail, &[], go_on, &[]);
        self.builder.switch_to_block(go_on);
    }

    fn block_for_bail(&mut self, at: usize) -> Block {
        let block = self.builder.create_block();
        self.pending.push((block, self.head + at));
        block
    }

    fn constant(&mut self, value: i64) -> Value {
        self.builder.ins().iconst(types::I64, value)
    }

    fn call(&mut self, function: FuncRef, args: &[Value]) -> Option<Value> {
        let call = self.builder.ins().call(function, args);
        self.builder.inst_results(call).first().copied()
    }

    #[allow(clippy::too_many_lines)]
    fn instruction(&mut self, at: usize, op: &Op) -> Option<()> {
        match op {
            Op::IntConstant { dst, value } => {
                let value = self.constant(*value);
                self.set(*dst, value)?;
            }
            Op::MoveInt { dst, src } => {
                let value = self.var(*src)?;
                self.set(*dst, value)?;
            }
            Op::Add { dst, a, b, .. }
            | Op::Subtract { dst, a, b, .. }
            | Op::Multiply { dst, a, b, .. } => {
                let (a, b) = (self.var(*a)?, self.var(*b)?);
                let (result, overflowed) = match op {
                    Op::Add { .. } => self.builder.ins().sadd_overflow(a, b),
                    Op::Subtract { .. } => self.builder.ins().ssub_overflow(a, b),
                    _ => self.builder.ins().smul_overflow(a, b),
                };
                self.bail_if(at, overflowed);
                self.set(*dst, result)?;
            }
            Op::Remainder { dst, a, b, .. } => {
                let (a, b) = (self.var(*a)?, self.var(*b)?);
                let zero = self.builder.ins().icmp_imm_s(IntCC::Equal, b, 0);
                let a_negative = self.builder.ins().icmp_imm_s(IntCC::SignedLessThan, a, 0);
                let b_negative = self.builder.ins().icmp_imm_s(IntCC::SignedLessThan, b, 0);
                let negative = self.builder.ins().bor(a_negative, b_negative);
                let unusual = self.builder.ins().bor(zero, negative);
                self.bail_if(at, unusual);
                let result = self.builder.ins().srem(a, b);
                self.set(*dst, result)?;
            }
            Op::Increment { dst } => {
                let value = self.var(*dst)?;
                let result = self.builder.ins().iadd_imm_s(value, 1);
                self.set(*dst, result)?;
            }
            Op::CompareInts { op, dst, a, b } => {
                let (a, b) = (self.var(*a)?, self.var(*b)?);
                let compared = self.builder.ins().icmp(condition(*op), a, b);
                let result = self.builder.ins().uextend(types::I64, compared);
                self.set(*dst, result)?;
            }
            Op::Not { dst, src } => {
                let value = self.var(*src)?;
                let compared = self.builder.ins().icmp_imm_s(IntCC::Equal, value, 0);
                let result = self.builder.ins().uextend(types::I64, compared);
                self.set(*dst, result)?;
            }
            Op::Jump { to } => {
                let block = self.block_for(*to);
                self.builder.ins().jump(block, &[]);
                return Some(());
            }
            Op::JumpIfFalse { condition, to } | Op::JumpIfTrue { condition, to } => {
                let value = self.var(*condition)?;
                let target = self.block_for(*to);
                let next = self.block_for(self.head + at + 1);
                if matches!(op, Op::JumpIfTrue { .. }) {
                    self.builder.ins().brif(value, target, &[], next, &[]);
                } else {
                    self.builder.ins().brif(value, next, &[], target, &[]);
                }
                return Some(());
            }
            Op::JumpIfAtLeast { a, b, to } => {
                let (a, b) = (self.var(*a)?, self.var(*b)?);
                let at_least = self
                    .builder
                    .ins()
                    .icmp(IntCC::SignedGreaterThanOrEqual, a, b);
                let target = self.block_for(*to);
                let next = self.block_for(self.head + at + 1);
                self.builder.ins().brif(at_least, target, &[], next, &[]);
                return Some(());
            }
            Op::CheckCount { src, .. } => {
                let value = self.var(*src)?;
                let negative = self
                    .builder
                    .ins()
                    .icmp_imm_s(IntCC::SignedLessThan, value, 0);
                self.bail_if(at, negative);
            }
            Op::NeverReached { .. } => {
                let bail = self.block_for_bail(at);
                self.builder.ins().jump(bail, &[]);
                return Some(());
            }
            _ => self.rational_or_table(at, op)?,
        }
        self.next(at);
        Some(())
    }

    fn rational_or_table(&mut self, at: usize, op: &Op) -> Option<()> {
        let (rationals, rationals_length) = (self.rationals, self.rationals_length);
        match op {
            Op::RationalConstant { dst, .. } => {
                let address = *self.constants.get(self.next_constant)?;
                self.next_constant += 1;
                let source = self.builder.ins().iconst(self.pointer_type, address);
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(i64::from(*dst)),
                    source,
                ];
                self.call(self.helpers.rational_copy, &args)?;
            }
            Op::MoveRational { dst, src } => {
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(i64::from(*dst)),
                    self.constant(i64::from(*src)),
                ];
                self.call(self.helpers.rational_move, &args)?;
            }
            Op::IntToRational { dst, src } => {
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(i64::from(*dst)),
                    self.var(*src)?,
                ];
                self.call(self.helpers.rational_from_int, &args)?;
            }
            Op::DivideInts { dst, a, b, .. } => {
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(i64::from(*dst)),
                    self.var(*a)?,
                    self.var(*b)?,
                ];
                let failed = self.call(self.helpers.rational_divide_ints, &args)?;
                self.bail_if(at, failed);
            }
            Op::AddRationals { dst, a, b, .. }
            | Op::SubtractRationals { dst, a, b, .. }
            | Op::MultiplyRationals { dst, a, b, .. }
            | Op::DivideRationals { dst, a, b, .. } => {
                let code = match op {
                    Op::AddRationals { .. } => 0,
                    Op::SubtractRationals { .. } => 1,
                    Op::MultiplyRationals { .. } => 2,
                    _ => 3,
                };
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(code),
                    self.constant(i64::from(*dst)),
                    self.constant(i64::from(*a)),
                    self.constant(i64::from(*b)),
                ];
                let failed = self.call(self.helpers.rational_arithmetic, &args)?;
                self.bail_if(at, failed);
            }
            Op::CompareRationals { op, dst, a, b } => {
                let args = [
                    rationals,
                    rationals_length,
                    self.constant(i64::from(*a)),
                    self.constant(i64::from(*b)),
                ];
                let ordering = self.call(self.helpers.rational_compare, &args)?;
                let compared = self.builder.ins().icmp_imm_s(condition(*op), ordering, 0);
                let result = self.builder.ins().uextend(types::I64, compared);
                self.set(*dst, result)?;
            }
            Op::Length { .. }
            | Op::Item { .. }
            | Op::ReadPosition { .. }
            | Op::MoveObject { .. } => {
                self.table(at, op)?;
            }
            _ => return None,
        }
        Some(())
    }

    fn table(&mut self, at: usize, op: &Op) -> Option<()> {
        let (objects, objects_length) = (self.objects, self.objects_length);
        match op {
            Op::MoveObject { dst, src } => {
                let args = [
                    self.objects,
                    self.objects_length,
                    self.constant(i64::from(*dst)),
                    self.constant(i64::from(*src)),
                ];
                let failed = self.call(self.helpers.object_move, &args)?;
                self.bail_if(at, failed);
                // The value `dst` held may be gone now, so its view is fetched again; the move is done, so a
                // failure here hands back after it.
                if self.views.contains_key(dst) {
                    self.fetch_view(*dst, self.head + at + 1)?;
                }
            }
            Op::Length { dst, table } if self.views.contains_key(table) => {
                let (_, length) = *self.views.get(table)?;
                let value = self.builder.use_var(length);
                self.set(*dst, value)?;
            }
            Op::Length { dst, table } => {
                let args = [objects, objects_length, self.constant(i64::from(*table))];
                let length = self.call(self.helpers.table_length, &args)?;
                let not_in_order = self
                    .builder
                    .ins()
                    .icmp_imm_s(IntCC::SignedLessThan, length, 0);
                self.bail_if(at, not_in_order);
                self.set(*dst, length)?;
            }
            Op::Item { dst, table, index } => {
                let item = self.var(*index)?;
                self.read_item(at, *dst, *table, item)?;
            }
            Op::ReadPosition {
                dst,
                table,
                index,
                counting,
                ..
            } => {
                let index = self.var(*index)?;
                let item = match counting {
                    Counting::Position => self.builder.ins().iadd_imm_s(index, -1),
                    Counting::Offset => index,
                };
                self.read_item(at, *dst, *table, item)?;
            }
            _ => return None,
        }
        Some(())
    }

    /// Reads the item at a zero-based index into a number register, handing the instruction back if it's missing.
    fn read_item(&mut self, at: usize, dst: Reg, table: u32, item: Value) -> Option<()> {
        if let (Reg::Int(reg) | Reg::Boolean(reg), Some(layout), Some((address, length))) =
            (dst, self.layout, self.views.get(&table).copied())
        {
            return self.read_item_in_place(at, dst, reg, layout, address, length, item);
        }
        let (objects, objects_length) = (self.objects, self.objects_length);
        let table = self.constant(i64::from(table));
        match dst {
            Reg::Int(dst) | Reg::Boolean(dst) => {
                let args = [objects, objects_length, table, item, self.out];
                let failed = self.call(self.helpers.table_int, &args)?;
                self.bail_if(at, failed);
                let value =
                    self.builder
                        .ins()
                        .load(types::I64, MemFlagsData::trusted(), self.out, 0);
                self.set(dst, value)
            }
            Reg::Rational(dst) => {
                let args = [
                    objects,
                    objects_length,
                    self.rationals,
                    self.rationals_length,
                    table,
                    item,
                    self.constant(i64::from(dst)),
                ];
                let failed = self.call(self.helpers.table_rational, &args)?;
                self.bail_if(at, failed);
                Some(())
            }
            Reg::Object(_) => None,
        }
    }
}

impl LoopBuilder<'_, '_> {
    /// Reads an int or boolean item straight from the table's memory: a bounds check, a tag check, a load. Anything
    /// unexpected hands the instruction back to tier 0.
    #[allow(clippy::too_many_arguments)]
    fn read_item_in_place(
        &mut self,
        at: usize,
        dst: Reg,
        reg: u32,
        layout: calls::ValueLayout,
        address: Variable,
        length: Variable,
        item: Value,
    ) -> Option<()> {
        let length = self.builder.use_var(length);
        let outside = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, item, length);
        self.bail_if(at, outside);
        let start = self.builder.use_var(address);
        let distance = self.builder.ins().imul_imm_s(item, layout.size);
        let place = self.builder.ins().iadd(start, distance);
        let tag = self
            .builder
            .ins()
            .load(types::I8, MemFlagsData::trusted(), place, 0);
        let expected = if matches!(dst, Reg::Boolean(_)) {
            layout.boolean_tag
        } else {
            layout.int_tag
        };
        let unexpected = self
            .builder
            .ins()
            .icmp_imm_u(IntCC::NotEqual, tag, i64::from(expected));
        self.bail_if(at, unexpected);
        let value = if matches!(dst, Reg::Boolean(_)) {
            let byte =
                self.builder
                    .ins()
                    .load(types::I8, MemFlagsData::trusted(), place, layout.payload);
            self.builder.ins().uextend(types::I64, byte)
        } else {
            self.builder
                .ins()
                .load(types::I64, MemFlagsData::trusted(), place, layout.payload)
        };
        self.set(reg, value)
    }
}

fn condition(op: Comparison) -> IntCC {
    match op {
        Comparison::Is => IntCC::Equal,
        Comparison::IsNot => IntCC::NotEqual,
        Comparison::AtLeast => IntCC::SignedGreaterThanOrEqual,
        Comparison::AtMost => IntCC::SignedLessThanOrEqual,
        Comparison::GreaterThan => IntCC::SignedGreaterThan,
        Comparison::LessThan => IntCC::SignedLessThan,
    }
}

/// A register's byte offset in the int file.
fn offset(reg: u32) -> Option<i32> {
    i32::try_from(reg).ok()?.checked_mul(8)
}

/// The int registers an instruction reads or writes, which the compiled loop keeps in variables.
fn int_registers(op: &Op) -> Vec<u32> {
    let number = |reg: &Reg| match reg {
        Reg::Int(reg) | Reg::Boolean(reg) => Some(*reg),
        Reg::Rational(_) | Reg::Object(_) => None,
    };
    match op {
        Op::IntConstant { dst, .. }
        | Op::Increment { dst }
        | Op::CompareRationals { dst, .. }
        | Op::Length { dst, .. } => vec![*dst],
        Op::MoveInt { dst, src } | Op::Not { dst, src } => vec![*dst, *src],
        Op::Add { dst, a, b, .. }
        | Op::Subtract { dst, a, b, .. }
        | Op::Multiply { dst, a, b, .. }
        | Op::Remainder { dst, a, b, .. }
        | Op::CompareInts { dst, a, b, .. } => vec![*dst, *a, *b],
        Op::JumpIfFalse { condition, .. } | Op::JumpIfTrue { condition, .. } => vec![*condition],
        Op::JumpIfAtLeast { a, b, .. } | Op::DivideInts { a, b, .. } => vec![*a, *b],
        Op::CheckCount { src, .. } | Op::IntToRational { src, .. } => vec![*src],
        Op::Item { dst, index, .. } | Op::ReadPosition { dst, index, .. } => {
            number(dst).into_iter().chain([*index]).collect()
        }
        _ => Vec::new(),
    }
}
