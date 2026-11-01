//! Finite interpretations of admitted opaque Type functions. These entries are
//! model evidence only, never equations granted to proof search or unification.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeApplication {
    pub head: DeclarationId,
    pub arguments: Vec<Term>,
    pub result: Term,
}
impl fmt::Display for TypeApplication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-application (head {}) (arguments", self.head)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (result {}))", self.result)
    }
}

pub(super) type Applications = HashMap<(DeclarationId, Vec<Term>), Term>;

pub(super) fn admitted(term: &Term, context: &Context) -> bool {
    matches!(term, Term::NormalForm { head, sort: Sort::Type, arguments }
        if context.congruence_heads.contains(head)
            && !context.tuple_heads.contains(head)
            && arguments.iter().all(|arg| argument_with_types(arg, &|_| true)))
}

/// Completed intrinsic value identities only. This is not program equality:
/// floating comparison, user Eq/Hash and unresolved value stores need their
/// own evidence and must not be guessed from Rust representation equality.
pub(super) fn fixed_argument(term: &Term) -> bool {
    argument_with_types(term, &|_| false)
}

/// A ground model packet may contain interpreted Type leaves. Pending source
/// Types are accepted only by application discovery, never witness replay.
pub(super) fn closed_argument(term: &Term, nominal: &BTreeSet<DeclarationId>) -> bool {
    argument_with_types(term, &|ty| closed_type(ty, nominal))
}

fn argument_with_types(term: &Term, type_leaf: &impl Fn(&Term) -> bool) -> bool {
    fn visit(
        term: &Term,
        depth: usize,
        remaining: &mut usize,
        type_leaf: &impl Fn(&Term) -> bool,
    ) -> bool {
        if depth >= 256 || *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        if term.sort() == Sort::Type {
            return type_leaf(term);
        }
        match term {
            Term::Unit
            | Term::Bool(_)
            | Term::Char(_)
            | Term::Integer(_)
            | Term::String(_)
            | Term::Symbol(_)
            | Term::BitVec(_) => true,
            // Formation is checked before PTC admission. A nominal payload
            // retains its declared Type and variant; an opaque NF domain does
            // not acquire collection/layout authority from this carrier alone.
            Term::NominalValue { ty, fields, .. } => {
                matches!(ty.as_ref(), Term::Nominal { .. })
                    && visit(ty, depth + 1, remaining, type_leaf)
                    && fields
                        .iter()
                        .all(|value| visit(value, depth + 1, remaining, type_leaf))
            }
            Term::OptionalValue { ty, value } => {
                matches!(ty.as_ref(), Term::Optional(_))
                    && visit(ty, depth + 1, remaining, type_leaf)
                    && value
                        .as_ref()
                        .is_none_or(|value| visit(value, depth + 1, remaining, type_leaf))
            }
            Term::ErrorValue { ty, label, value } => {
                matches!(ty.as_ref(), Term::ErrorResult { .. })
                    && visit(ty, depth + 1, remaining, type_leaf)
                    && label
                        .as_ref()
                        .is_none_or(|label| visit(label, depth + 1, remaining, type_leaf))
                    && visit(value, depth + 1, remaining, type_leaf)
            }
            Term::TupleValue(values) => values
                .iter()
                .all(|value| visit(value, depth + 1, remaining, type_leaf)),
            Term::RecordValue(fields) => {
                let mut labels = BTreeSet::new();
                fields.iter().all(|(label, value)| {
                    labels.insert(label) && visit(value, depth + 1, remaining, type_leaf)
                })
            }
            _ => false,
        }
    }
    // Inputs and model copies are separately charged against the query budget.
    // Bound this shape-only predicate too: it must not recursively inspect an
    // unbounded host packet before those admission checks can reject it.
    visit(term, 0, &mut 100_000, type_leaf)
}

pub(super) fn index(
    entries: &[TypeApplication],
    context: &Context,
    fuel: &mut usize,
    depth: usize,
) -> Result<Applications, ProofError> {
    let mut result = HashMap::new();
    for entry in entries {
        *fuel = fuel.checked_sub(1).ok_or(ProofError::BudgetExhausted)?;
        if !context.congruence_heads.contains(&entry.head)
            || context.tuple_heads.contains(&entry.head)
        {
            return Err(ProofError::Invalid);
        }
        for (value, is_argument) in entry
            .arguments
            .iter()
            .map(|value| (value, true))
            .chain(std::iter::once((&entry.result, false)))
        {
            if !within_input_limits(Input::Term(value), depth, fuel) {
                return Err(ProofError::BudgetExhausted);
            }
            if !(closed_type(value, &context.nominal_constructors)
                || (is_argument && closed_argument(value, &context.nominal_constructors)))
                || value.check_scope(&[]).is_err()
            {
                return Err(ProofError::Invalid);
            }
        }
        if result
            .insert((entry.head, entry.arguments.clone()), entry.result.clone())
            .is_some()
        {
            return Err(ProofError::Invalid);
        }
    }
    Ok(result)
}

pub(super) fn charge(value: &Term, fuel: &std::cell::Cell<usize>) -> Option<()> {
    let mut remaining = fuel.get();
    let result = within_input_limits(Input::Term(value), 256, &mut remaining);
    fuel.set(if result { remaining } else { 0 });
    result.then_some(())
}
pub(super) fn copy(value: &Term, fuel: &std::cell::Cell<usize>) -> Option<Term> {
    charge(value, fuel)?;
    Some(value.clone())
}
