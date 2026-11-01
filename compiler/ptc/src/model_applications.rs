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
            && arguments.iter().all(|arg| arg.sort() == Sort::Type))
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
        for value in entry.arguments.iter().chain(std::iter::once(&entry.result)) {
            if !within_input_limits(Input::Term(value), depth, fuel) {
                return Err(ProofError::BudgetExhausted);
            }
            if !closed_type(value, &context.nominal_constructors) || value.check_scope(&[]).is_err()
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
