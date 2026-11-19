//! Checked initialization programs, distinct from completed associated values.
//! Formation, protocol evidence and full phase checking remain caller duties.
use crate::{
    DeclarationParameter, Term,
    program::{Expr, ExprKind as K},
    value_admission::Error,
};
use std::fmt;

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

impl MemberSource<'_> {
    pub fn expression(&self) -> &Expr {
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
    expression: Expr,
}
impl AssociatedInitializer {
    pub fn new(expression: Expr, fuel: &mut usize) -> Result<Self, Error> {
        if !crate::free_bindings::collect(&expression, fuel)?.is_empty() {
            return Err("associated initializer cannot capture caller locals".into());
        }
        check_calls(&expression, fuel, 0)?;
        Ok(Self { expression })
    }
    pub fn expression(&self) -> &Expr {
        &self.expression
    }
    pub fn into_expression(self) -> Expr {
        self.expression
    }

    /// Substitution preserves a program; it does not certify a completed value.
    /// The caller must establish declaration argument domains before invocation.
    pub fn instantiate(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        crate::copy_budget::charge_substitution(&self.expression, parameters, arguments, fuel)
            .map_err(|_| Error::Budget)?;
        let expression = self
            .expression
            .instantiate_parameters(parameters, arguments)
            .map_err(|e| e.to_string())?;
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
