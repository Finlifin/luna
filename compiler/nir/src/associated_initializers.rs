//! Checked initialization programs, distinct from completed associated values.
//! Formation, protocol evidence and full phase checking remain caller duties.
use crate::{
    DeclarationSubstitution, Term,
    program::{Expr, ExprKind as K},
    value_admission::Error,
};
use std::{fmt, sync::Arc};

/// Source-owned programs, indexed by implementation and member. Selecting a
/// row grants no authority to execute a template from this directory.
pub type MemberPrograms = std::collections::BTreeMap<
    crate::DeclarationId,
    std::collections::BTreeMap<crate::DeclarationId, AssociatedInitializer>,
>;

/// Iterate executable sources in the implementation's lexical frame. A
/// captured source moves each template into exactly one of these locations.
pub fn programs_for<'a>(
    row: &'a crate::program::Implementation,
    captured: Option<&'a MemberPrograms>,
) -> impl Iterator<Item = (&'a crate::DeclarationId, &'a AssociatedInitializer)> {
    row.associated_initializers
        .iter()
        .map(|(id, template)| (id, template))
        .chain(
            captured
                .and_then(|programs| programs.get(&row.declaration))
                .into_iter()
                .flatten(),
        )
}

/// A concrete selection whose declaration source has yet to supply the member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssociatedSourceDemand {
    pub implementation: crate::DeclarationId,
    pub arguments: Vec<Term>,
    pub member: crate::DeclarationId,
    pub result: Term,
}

impl fmt::Display for AssociatedSourceDemand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-source-demand (implementation {}) (arguments",
            self.implementation
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (member {}) (result {}))", self.member, self.result)
    }
}

/// Declaration storage distinguishes executable programs from completed data.
/// This view borrows the template before any implementation substitution.
pub enum MemberSource<'a> {
    Value(&'a Expr),
    Initializer(&'a AssociatedInitializer),
}

impl<'a> MemberSource<'a> {
    pub fn expression(&self) -> &'a Expr {
        match self {
            Self::Value(value) => value,
            Self::Initializer(template) => template.expression(),
        }
    }
}

impl crate::program::Implementation {
    pub fn associated_source(
        &self,
        member: crate::DeclarationId,
        fuel: &mut usize,
    ) -> Result<Option<MemberSource<'_>>, Error> {
        let entries = self
            .associated
            .iter()
            .map(|(id, value)| (id, MemberSource::Value(value)))
            .chain(
                self.associated_initializers
                    .iter()
                    .map(|(id, template)| (id, MemberSource::Initializer(template))),
            );
        let mut found = None;
        for (id, entry) in entries {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if *id == member && found.replace(entry).is_some() {
                return Err("selected implementation has duplicate associated values".into());
            }
        }
        Ok(found)
    }
}

/// What selecting an associated member supplies after substitution.
#[derive(Clone, Debug, PartialEq)]
pub enum AssociatedSelection {
    Completed(crate::completed_values::CompletedValue),
    Initializer(AssociatedInitializer),
}

impl AssociatedSelection {
    pub fn expression(&self) -> &Expr {
        match self {
            Self::Completed(value) => value.expression(),
            Self::Initializer(initializer) => initializer.expression(),
        }
    }
}

impl fmt::Display for AssociatedSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed(value) => write!(
                f,
                "(associated-selection (completed {}))",
                value.expression()
            ),
            Self::Initializer(initializer) => write!(f, "(associated-selection {initializer})"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssociatedInitializer {
    expression: Arc<Expr>,
}
/// Attribution to an immutable checked initializer without retaining its body.
#[derive(Clone, Debug)]
pub struct ProgramIdentity(std::sync::Weak<Expr>);

impl PartialEq for ProgramIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl AssociatedInitializer {
    pub fn identity(&self) -> ProgramIdentity {
        ProgramIdentity(Arc::downgrade(&self.expression))
    }

    pub fn new(expression: Expr, fuel: &mut usize) -> Result<Self, Error> {
        if !crate::free_bindings::collect(&expression, fuel)?.is_empty() {
            return Err("associated initializer cannot capture caller locals".into());
        }
        check_calls(&expression, fuel, 0)?;
        Ok(Self {
            expression: Arc::new(expression),
        })
    }
    pub fn expression(&self) -> &Expr {
        &self.expression
    }
    pub fn into_shared_expression(self) -> Arc<Expr> {
        self.expression
    }

    /// Detach syntax for an owning IR field. A freshly specialized program can
    /// move directly; copying a shared template requires the caller's budget.
    pub fn into_expression(self, steps: &mut u64) -> Result<Expr, Error> {
        match Arc::try_unwrap(self.expression) {
            Ok(body) => Ok(body),
            Err(shared) => {
                crate::copy_budget::charge_expression(&shared, steps).map_err(|_| Error::Budget)?;
                Ok((*shared).clone())
            }
        }
    }

    pub fn instantiate(
        &self,
        substitution: &DeclarationSubstitution<'_>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let expression = substitution
            .instantiate_expression_bounded(&self.expression, fuel)
            .map_err(crate::type_computations::Error::from)?;
        Self::new(expression, fuel)
    }
}
fn check_calls(expression: &Expr, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Budget);
    }
    crate::visit::expression_pruned_checked(expression, fuel, |node, fuel| {
        if let K::Call { callee, .. } = &node.kind {
            if !callee.ty.is_comptime_callable() {
                return Err("associated initializer cannot invoke a runtime callable".into());
            }
        }
        if let K::Closure { captures, .. } = &node.kind {
            // Constructing a closure evaluates captures, not its body. The body
            // is checked independently at its declared phase.
            for capture in captures {
                if let Some(value) = &capture.value {
                    check_calls(value, fuel, depth + 1)?;
                }
            }
            return Ok(false);
        }
        Ok(true)
    })
}
impl fmt::Display for AssociatedInitializer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-initializer (result {}) (body {}))",
            self.expression.ty, self.expression
        )
    }
}
