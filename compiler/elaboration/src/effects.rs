//! Bounded lexical routing for compiler-selected effect entries. This checks
//! label/type identity only: coverage, parameter/result typing, program state,
//! ownership, continuation storage and runtime supply are separate obligations.
use crate::ptc::{Budget, Context, Formula, Outcome};
use nir::effects::{Abi, Entry, Frame, Operation};
use nir::{Sort, Term};
use std::fmt;

mod requests;
pub use requests::Request;

mod rows;
pub use rows::{RowEmbedding, RowLayer, embed_row, select_layer};

mod instances;
pub use instances::{Instance, instantiate, instantiate_with_domains, instantiate_with_evidence};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    ArgumentCount { expected: usize, actual: usize },
    ArgumentType { index: usize },
    FormationNotEstablished,
    MissingEntry,
    PendingIdentity,
    AbiMismatch,
    AmbiguousInstallation,
    BudgetExhausted,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArgumentCount { expected, actual } => {
                return write!(
                    f,
                    "effect request argument count mismatch: expected {expected}, found {actual}"
                );
            }
            Self::ArgumentType { index } => {
                return write!(f, "effect request argument {index} has an unproved type");
            }
            _ => {}
        }
        f.write_str(match self {
            Self::ArgumentCount { .. } | Self::ArgumentType { .. } => unreachable!(),
            Self::FormationNotEstablished => {
                "effect operation formation contract is not established"
            }
            Self::InvalidInput => "effect operation input has an invalid type shape or scope",
            Self::MissingEntry => "effect request has no complete handler route",
            Self::PendingIdentity => "effect operation identity needs further evidence",
            Self::AbiMismatch => "effect handler entry uses an incompatible ABI",
            Self::AmbiguousInstallation => {
                "multiple dispatch entries match one operation in an installation"
            }
            Self::BudgetExhausted => "effect routing budget exhausted",
        })
    }
}
impl std::error::Error for Error {}

/// Borrowed inputs prevent a route from surviving mutated frames or type facts.
/// This is a lexical routing plan, not a completed executable evidence object.
#[derive(Debug)]
pub struct Route<'a> {
    frames: &'a [Frame],
    _context: &'a Context,
    hops: Vec<(usize, usize)>,
}
impl<'a> Route<'a> {
    pub fn entries(&self) -> impl Iterator<Item = &'a Entry> + '_ {
        self.hops
            .iter()
            .map(|&(frame, entry)| &self.frames[frame].entries[entry])
    }
    /// The branch runs outside its entire installation, including sibling
    /// operations. The handled computation keeps the full installed chain.
    pub fn branch_environment(&self, hop: usize) -> Option<&'a [Frame]> {
        self.hops.get(hop).map(|&(frame, _)| &self.frames[..frame])
    }
}
impl fmt::Display for Route<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effect-route (hops")?;
        for &(frame, entry) in &self.hops {
            write!(
                f,
                " (hop (frame {frame}) (entry {entry}) (outer-prefix {frame}))"
            )?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Copy)]
enum Relation {
    Same,
    Different,
    Pending,
}

// PTC is used exclusively for Type-sort equality. Value/runtime input identity
// needs its own evidence; lack of structural equality never proves inequality.
fn relation(a: &Operation, b: &Operation, context: &Context, budget: Budget) -> Relation {
    if a.declaration != b.declaration || a.arguments.len() != b.arguments.len() {
        return Relation::Different;
    }
    let mut pending = false;
    let per_argument = Budget {
        steps: budget.steps / a.arguments.len().max(1) / 2,
        max_depth: budget.max_depth.min(64),
    };
    for (a, b) in a.arguments.iter().zip(&b.arguments) {
        if a == b {
            continue;
        }
        if a.sort() != b.sort() {
            return Relation::Different;
        }
        if a.sort() != Sort::Type {
            if matches!(
                (a, b),
                (Term::Bool(_), Term::Bool(_))
                    | (Term::Char(_), Term::Char(_))
                    | (Term::String(_), Term::String(_))
                    | (Term::Symbol(_), Term::Symbol(_))
                    | (Term::Integer(_), Term::Integer(_))
                    | (Term::Float(_), Term::Float(_))
                    | (Term::BitVec(_), Term::BitVec(_))
                    | (Term::Float16(_), Term::Float16(_))
                    | (Term::BFloat16(_), Term::BFloat16(_))
                    | (Term::Float32(_), Term::Float32(_))
                    | (Term::Float64(_), Term::Float64(_))
            ) {
                return Relation::Different;
            }
            pending = true;
            continue;
        }
        let equality = Formula::equal(a.clone(), b.clone());
        if matches!(
            context.prove(equality.clone(), per_argument),
            Outcome::Entailed(_)
        ) {
            continue;
        }
        if matches!(
            context.prove(equality.negate(), per_argument),
            Outcome::Entailed(_)
        ) {
            return Relation::Different;
        }
        pending = true;
    }
    if pending {
        Relation::Pending
    } else {
        Relation::Same
    }
}

/// Every inspected term must have bounded width/depth before recursive Eq,
/// Clone or PTC. `children()` is not used on an unbounded-width input.
fn charge(term: &Term, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    if depth == 0 || *fuel == 0 {
        return Err(Error::BudgetExhausted);
    }
    *fuel -= 1;
    fn bytes(fuel: &mut usize, amount: usize) -> Result<(), Error> {
        *fuel = fuel.checked_sub(amount).ok_or(Error::BudgetExhausted)?;
        Ok(())
    }
    fn terms(items: &[Term], fuel: &mut usize, depth: usize) -> Result<(), Error> {
        if items.len() > *fuel {
            return Err(Error::BudgetExhausted);
        }
        for item in items {
            charge(item, fuel, depth)?;
        }
        Ok(())
    }
    match term {
        Term::ErrorResult { errors, result } => {
            terms(errors, fuel, depth - 1)?;
            charge(result, fuel, depth - 1)
        }
        Term::Computation { effects, result } => {
            if effects.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            *fuel -= effects.len();
            charge(result, fuel, depth - 1)?;
            for operation in effects {
                terms(&operation.arguments, fuel, depth - 1)?;
            }
            Ok(())
        }
        Term::ClosureContextType
        | Term::Primitive(_)
        | Term::Rigid { .. }
        | Term::Bound { .. }
        | Term::Hole { .. }
        | Term::Runtime(_)
        | Term::Unit
        | Term::Bool(_)
        | Term::Char(_)
        | Term::Float16(_)
        | Term::BFloat16(_)
        | Term::Float32(_)
        | Term::Float64(_)
        | Term::Value { .. } => Ok(()),
        Term::String(value) | Term::Symbol(value) => bytes(fuel, value.len()),
        Term::Integer(value) => bytes(fuel, value.bit_len().div_ceil(32)),
        Term::Float(value) => bytes(
            fuel,
            value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32)),
        ),
        Term::BitVec(value) => bytes(fuel, value.words().len()),
        Term::Nominal { arguments, .. }
        | Term::ClosureSnapshot { arguments, .. }
        | Term::FunctionValue { arguments, .. }
        | Term::NormalForm { arguments, .. }
        | Term::Tuple(arguments)
        | Term::ClosureContextValue(arguments)
        | Term::TypeListValue(arguments)
        | Term::TupleValue(arguments) => terms(arguments, fuel, depth - 1),
        Term::AssociatedType {
            subject, arguments, ..
        } => {
            charge(subject, fuel, depth - 1)?;
            terms(arguments, fuel, depth - 1)
        }
        Term::ContextualClosure { context, signature } => {
            charge(context, fuel, depth - 1)?;
            charge(signature, fuel, depth - 1)
        }
        Term::Closure {
            environment,
            signature,
        } => {
            charge(signature, fuel, depth - 1)?;
            terms(environment, fuel, depth - 1)
        }
        Term::Optional(inner)
        | Term::Pointer(inner)
        | Term::ComptimeFunction(inner)
        | Term::NormalFormFunction(inner, _)
        | Term::UnsafeFunction(inner) => charge(inner, fuel, depth - 1),
        Term::Function { parameters, result } => {
            if parameters.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            for parameter in parameters {
                charge(&parameter.ty, fuel, depth - 1)?;
            }
            charge(result, fuel, depth - 1)
        }
        Term::OptionalValue { ty, value } => {
            charge(ty, fuel, depth - 1)?;
            if let Some(value) = value {
                charge(value, fuel, depth - 1)?;
            }
            Ok(())
        }
        Term::ErrorValue { ty, label, value } => {
            charge(ty, fuel, depth - 1)?;
            if let Some(label) = label {
                charge(label, fuel, depth - 1)?;
            }
            charge(value, fuel, depth - 1)
        }
        Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
            if fields.len().saturating_add(1) > *fuel {
                return Err(Error::BudgetExhausted);
            }
            charge(ty, fuel, depth - 1)?;
            for field in fields {
                charge(field, fuel, depth - 1)?;
            }
            Ok(())
        }
        Term::Record(fields) | Term::RecordValue(fields) => {
            if fields.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            for (name, ty) in fields {
                bytes(fuel, name.len())?;
                charge(ty, fuel, depth - 1)?;
            }
            Ok(())
        }
    }
}

/// Resolve a request through lexical installations, nearest first. Unknown
/// nearer identity blocks selection; it cannot be treated as an absent entry.
/// Partial entries retain outer fallbacks and require a total terminal entry.
pub fn route<'a>(
    request: &Operation,
    abi: Abi,
    frames: &'a [Frame],
    context: &'a Context,
    budget: Budget,
) -> Result<Route<'a>, Error> {
    let mut fuel = budget.steps;
    let depth = budget.max_depth.min(64);
    let mut count = frames.len();
    if count >= fuel {
        return Err(Error::BudgetExhausted);
    }
    for frame in frames {
        count = count
            .checked_add(frame.entries.len())
            .ok_or(Error::BudgetExhausted)?;
        if count >= fuel {
            return Err(Error::BudgetExhausted);
        }
    }
    if count >= fuel {
        return Err(Error::BudgetExhausted);
    }
    fuel -= count;
    for argument in &request.arguments {
        charge(argument, &mut fuel, depth)?;
        argument.check_scope(&[]).map_err(|_| Error::InvalidInput)?;
    }
    for frame in frames {
        for entry in &frame.entries {
            for argument in &entry.operation.arguments {
                charge(argument, &mut fuel, depth)?;
                argument.check_scope(&[]).map_err(|_| Error::InvalidInput)?;
            }
        }
    }
    let per_entry = Budget {
        steps: fuel / count.max(1),
        max_depth: depth,
    };
    let mut hops = Vec::new();
    for (frame_index, frame) in frames.iter().enumerate().rev() {
        let mut selected = None;
        for (entry_index, entry) in frame.entries.iter().enumerate() {
            match relation(request, &entry.operation, context, per_entry) {
                Relation::Different => continue,
                Relation::Pending => return Err(Error::PendingIdentity),
                Relation::Same => {
                    if selected.replace(entry_index).is_some() {
                        return Err(Error::AmbiguousInstallation);
                    }
                    if entry.abi != abi {
                        return Err(Error::AbiMismatch);
                    }
                }
            }
        }
        if let Some(entry_index) = selected {
            hops.push((frame_index, entry_index));
            if frame.entries[entry_index].total {
                return Ok(Route {
                    frames,
                    _context: context,
                    hops,
                });
            }
        }
    }
    Err(Error::MissingEntry)
}
