//! Interface identities and their instantiated signatures, without executable trees.
use crate::{Error, copy_error};
use nir::program::{Expr, ExprKind as K, SourceRange};
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Debug)]
pub struct Method {
    pub interface: DeclarationId,
    pub member: DeclarationId,
    pub subject: Term,
    pub interface_arguments: Vec<Term>,
    pub method_arguments: Vec<Term>,
    pub output: Term,
    pub source: SourceRange,
}
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub interface: DeclarationId,
    pub member: DeclarationId,
    pub subject: &'a Term,
    pub interface_arguments: &'a [Term],
    pub method_arguments: &'a [Term],
    pub output: &'a Term,
    pub source: SourceRange,
}
impl<'a> View<'a> {
    pub fn expression(expr: &'a Expr) -> Option<Self> {
        let K::TraitMethod {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
        } = &expr.kind
        else {
            return None;
        };
        Some(Self {
            interface: *interface,
            member: *member,
            subject,
            interface_arguments,
            method_arguments,
            output: &expr.ty,
            source: expr.source,
        })
    }
}
impl Method {
    pub fn view(&self) -> View<'_> {
        View {
            interface: self.interface,
            member: self.member,
            subject: &self.subject,
            interface_arguments: &self.interface_arguments,
            method_arguments: &self.method_arguments,
            output: &self.output,
            source: self.source,
        }
    }
    pub(crate) fn copy(expr: &Expr, fuel: &mut usize) -> Result<Self, Error> {
        let view = View::expression(expr).ok_or(Error::Unsupported(expr.source))?;
        nir::copy_budget::charge_substitution(expr, &[], &[], fuel)
            .map_err(|error| copy_error(error, expr.source))?;
        Ok(Self {
            interface: view.interface,
            member: view.member,
            subject: view.subject.clone(),
            interface_arguments: view.interface_arguments.to_vec(),
            method_arguments: view.method_arguments.to_vec(),
            output: view.output.clone(),
            source: view.source,
        })
    }
}
impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(trait-method {} (member {}) (subject {}) (interface-arguments",
            self.interface, self.member, self.subject
        )?;
        for argument in &self.interface_arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (method-arguments")?;
        for argument in &self.method_arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (output {}) (source {}))", self.output, self.source)
    }
}
