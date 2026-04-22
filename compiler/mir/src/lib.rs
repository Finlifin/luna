//! # MIR (Mid-level Intermediate Representation)
//!
//! A control-flow-graph-based IR, lowered from the typed HIR. Each
//! function body is represented as a set of basic blocks, each ending
//! in a terminator that encodes branching and control flow.
//!
//! Modeled after rustc's MIR.

pub mod display;

use std::fmt;

use hir::common::{BinOp, UnOp};
use hir::hir_id::LocalDefId;
use ty::Ty;

// ── Index types ──────────────────────────────────────────────────────────────

/// Index of a local variable / temporary in a MIR body.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Local(pub u32);

impl Local {
    /// The return place: `_0`.
    pub const RETURN_PLACE: Local = Local(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn new(index: u32) -> Self {
        Local(index)
    }
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.0)
    }
}

impl fmt::Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.0)
    }
}

/// Index of a basic block in a MIR body.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BasicBlock(pub u32);

impl BasicBlock {
    pub const START_BLOCK: BasicBlock = BasicBlock(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn new(index: u32) -> Self {
        BasicBlock(index)
    }
}

impl fmt::Debug for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

impl fmt::Display for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

// ── Body ─────────────────────────────────────────────────────────────────────

/// A single MIR function body.
pub struct Body<'tcx> {
    /// The definition this body belongs to.
    pub source: LocalDefId,
    /// The basic blocks that make up this function.
    pub basic_blocks: Vec<BasicBlockData<'tcx>>,
    /// Declarations for all local variables (including return place,
    /// arguments, and temporaries).
    pub local_decls: Vec<LocalDecl<'tcx>>,
    /// Number of function arguments (not counting the return place).
    /// The arguments occupy locals `_1` through `_arg_count`.
    pub arg_count: usize,
}

impl<'tcx> Body<'tcx> {
    pub fn new(source: LocalDefId, arg_count: usize) -> Self {
        Body {
            source,
            basic_blocks: Vec::new(),
            local_decls: Vec::new(),
            arg_count,
        }
    }

    /// Allocate a new local variable. Returns the `Local` index.
    pub fn push_local(&mut self, decl: LocalDecl<'tcx>) -> Local {
        let idx = self.local_decls.len() as u32;
        self.local_decls.push(decl);
        Local::new(idx)
    }

    /// Allocate a new basic block. Returns the `BasicBlock` index.
    pub fn push_block(&mut self, data: BasicBlockData<'tcx>) -> BasicBlock {
        let idx = self.basic_blocks.len() as u32;
        self.basic_blocks.push(data);
        BasicBlock::new(idx)
    }

    /// Create an empty block and return its index.
    pub fn new_block(&mut self) -> BasicBlock {
        self.push_block(BasicBlockData::new())
    }
}

/// Declaration of a local variable.
pub struct LocalDecl<'tcx> {
    pub ty: Ty<'tcx>,
    pub name: Option<String>,
}

// ── BasicBlockData ───────────────────────────────────────────────────────────

/// A basic block: a sequence of statements followed by a terminator.
pub struct BasicBlockData<'tcx> {
    pub statements: Vec<Statement<'tcx>>,
    pub terminator: Option<Terminator<'tcx>>,
}

impl<'tcx> BasicBlockData<'tcx> {
    pub fn new() -> Self {
        BasicBlockData {
            statements: Vec::new(),
            terminator: None,
        }
    }
}

// ── Statement ────────────────────────────────────────────────────────────────

pub struct Statement<'tcx> {
    pub kind: StatementKind<'tcx>,
}

pub enum StatementKind<'tcx> {
    /// `place = rvalue`
    Assign(Place, Rvalue<'tcx>),
    /// No-op (placeholder).
    Nop,
}

// ── Terminator ───────────────────────────────────────────────────────────────

pub struct Terminator<'tcx> {
    pub kind: TerminatorKind<'tcx>,
}

pub enum TerminatorKind<'tcx> {
    /// Unconditional jump.
    Goto { target: BasicBlock },

    /// Branch on an integer discriminant.
    SwitchInt {
        discr: Operand<'tcx>,
        /// (value, target) pairs, plus a fallback "otherwise" block.
        targets: SwitchTargets,
    },

    /// Return from the function. The return value is read from `_0`.
    Return,

    /// Function call.
    Call {
        /// The function being called (usually a constant referring to a fn def).
        func: Operand<'tcx>,
        /// Arguments to the function.
        args: Vec<Operand<'tcx>>,
        /// Where to write the return value.
        destination: Place,
        /// Block to jump to after the call returns.
        target: Option<BasicBlock>,
    },

    /// Indicates unreachable code.
    Unreachable,
}

/// A set of switch targets: pairs of (test_value, block) plus an otherwise block.
pub struct SwitchTargets {
    /// `(value, target)` pairs.
    pub values: Vec<(i64, BasicBlock)>,
    /// The "otherwise" / default block.
    pub otherwise: BasicBlock,
}

// ── Place ────────────────────────────────────────────────────────────────────

/// A memory location: a local variable potentially followed by
/// projections (field access, deref, index).
#[derive(Clone, Debug)]
pub struct Place {
    pub local: Local,
    pub projection: Vec<PlaceElem>,
}

impl Place {
    pub fn local(local: Local) -> Self {
        Place {
            local,
            projection: Vec::new(),
        }
    }

    /// Return place: `_0`.
    pub fn return_place() -> Self {
        Place::local(Local::RETURN_PLACE)
    }
}

/// A single step of a place projection.
#[derive(Clone, Debug)]
pub enum PlaceElem {
    /// Field access by index.
    Field(u32),
    /// Pointer dereference.
    Deref,
    /// Array indexing.
    Index(Local),
}

// ── Operand ──────────────────────────────────────────────────────────────────

/// An operand in an rvalue or terminator.
pub enum Operand<'tcx> {
    /// Copy the value from a place (for Copy types).
    Copy(Place),
    /// Move the value from a place.
    Move(Place),
    /// An inline constant.
    Constant(Constant<'tcx>),
}

// ── Rvalue ───────────────────────────────────────────────────────────────────

/// A right-hand-side value in an assignment.
pub enum Rvalue<'tcx> {
    /// Directly use an operand.
    Use(Operand<'tcx>),
    /// Binary operation.
    BinaryOp(BinOp, Operand<'tcx>, Operand<'tcx>),
    /// Unary operation.
    UnaryOp(UnOp, Operand<'tcx>),
    /// Create a reference to a place.
    Ref(Place),
    /// Aggregate construction (tuple, struct, array).
    Aggregate(AggregateKind, Vec<Operand<'tcx>>),
}

/// What kind of aggregate is being constructed.
pub enum AggregateKind {
    Tuple,
    Array,
    Adt(LocalDefId),
}

// ── Constant ─────────────────────────────────────────────────────────────────

/// An embedded constant value.
pub struct Constant<'tcx> {
    pub ty: Ty<'tcx>,
    pub kind: ConstKind,
}

pub enum ConstKind {
    /// An integer literal.
    Int(i64),
    /// A float literal.
    Float(f64),
    /// A boolean literal.
    Bool(bool),
    /// A character literal.
    Char(char),
    /// A string literal.
    Str(String),
    /// Reference to a function definition by DefId.
    FnDef(LocalDefId),
    /// Reference to a function by name (before full resolution).
    FnName(String),
    /// The unit value `()`.
    Unit,
    /// A null pointer constant.
    Null,
}
