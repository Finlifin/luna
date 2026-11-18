//! Callable identities in one immutable source world.
use crate::{DeclarationId, Term, phases::Error, program::*};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy)]
enum Slot {
    Function(usize),
    Quote { index: usize, staged: bool },
    Runtime(usize),
}

#[derive(Default)]
pub(crate) struct Positions(BTreeMap<DeclarationId, Slot>);

#[derive(Clone, Copy)]
pub struct Declaration<'a> {
    pub interface: &'a FunctionDeclaration,
    pub source: SourceRange,
}

impl std::fmt::Display for Declaration<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(source-declaration (source {} {}) {})",
            self.source.start, self.source.end, self.interface
        )
    }
}

#[derive(Clone, Default)]
pub struct Callables<'a> {
    functions: &'a [Function],
    quotes: &'a [Function],
    runtime: &'a [RuntimeFunctionIdentity],
    positions: Arc<Positions>,
}

impl Positions {
    pub(crate) fn build(package: &Package, fuel: &mut usize) -> Result<Arc<Self>, Error> {
        let source = SourceRange { start: 0, end: 0 };
        let count = package
            .functions
            .len()
            .saturating_add(package.quote_templates.len())
            .saturating_add(package.runtime_function_identities.len());
        *fuel = fuel
            .checked_sub(count)
            .ok_or_else(|| Error::budget(source, "callable directory budget exhausted"))?;
        let mut entries = BTreeMap::new();
        let mut insert = |id, slot, at, diagnostic| {
            if entries.insert(id, slot).is_some() {
                return Err(Error::invalid(at, diagnostic));
            }
            Ok(())
        };
        for (i, f) in package.functions.iter().enumerate() {
            insert(
                f.declaration,
                Slot::Function(i),
                f.body.source,
                "duplicate function declaration identity",
            )?;
        }
        for (i, f) in package.runtime_function_identities.iter().enumerate() {
            if !matches!(f.signature.callable_signature(), Term::Function { .. })
                || f.signature.is_comptime_callable()
            {
                return Err(Error::invalid(
                    source,
                    "invalid or duplicate runtime function identity",
                ));
            }
            insert(
                f.declaration,
                Slot::Runtime(i),
                source,
                "invalid or duplicate runtime function identity",
            )?;
        }
        for (i, f) in package.quote_templates.iter().enumerate() {
            if f.phase != FunctionPhase::Runtime {
                return Err(Error::invalid(
                    f.body.source,
                    "invalid or duplicate quote template identity",
                ));
            }
            let parameters = validate_frame(
                crate::contracts::CallableProgram::declaration(&f.interface, f.body.source),
                true,
                fuel,
            )?;
            let staged = parameters
                .iter()
                .any(|parameter| parameter.sort != crate::Sort::Runtime);
            insert(
                f.declaration,
                Slot::Quote { index: i, staged },
                f.body.source,
                "invalid or duplicate quote template identity",
            )?;
        }
        for f in &package.functions {
            validate_frame(
                crate::contracts::CallableProgram::declaration(&f.interface, f.body.source),
                false,
                fuel,
            )?;
        }
        for method in package
            .traits
            .iter()
            .flat_map(|interface| &interface.methods)
        {
            use crate::code_interfaces::MethodSignature::*;
            use crate::phases::contextual::validate_method_frame;
            match &method.signature {
                Callable(ty) => {
                    validate_frame(
                        crate::contracts::CallableProgram::ordinary_method(method, ty),
                        false,
                        fuel,
                    )?;
                }
                Generator(sig) => validate_method_frame(method, sig, fuel)?,
                PatternGenerator(sig) => validate_method_frame(method, sig, fuel)?,
                DataConsumer(sig) => validate_method_frame(method, sig, fuel)?,
            }
        }
        Ok(Arc::new(Self(entries)))
    }

    pub(crate) fn bind<'a>(self: &Arc<Self>, package: &'a Package) -> Callables<'a> {
        Callables {
            functions: &package.functions,
            quotes: &package.quote_templates,
            runtime: &package.runtime_function_identities,
            positions: self.clone(),
        }
    }
}

impl<'a> Callables<'a> {
    pub fn new(package: &'a Package, fuel: &mut usize) -> Result<Self, Error> {
        Ok(Positions::build(package, fuel)?.bind(package))
    }

    pub fn shares_directory(&self, other: &Callables<'_>) -> bool {
        Arc::ptr_eq(&self.positions, &other.positions)
    }

    fn function(&self, id: &DeclarationId) -> Option<&'a Function> {
        let Slot::Function(i) = *self.positions.0.get(id)? else {
            return None;
        };
        Some(&self.functions[i])
    }

    pub fn declaration(&self, id: &DeclarationId) -> Option<Declaration<'a>> {
        self.function(id).map(|f| Declaration {
            interface: &f.interface,
            source: f.body.source,
        })
    }

    pub fn body(&self, id: &DeclarationId) -> Option<&'a Expr> {
        self.function(id).map(|f| &f.body)
    }

    pub fn interface(&self, id: &DeclarationId) -> Option<&'a FunctionDeclaration> {
        self.function(id).map(|f| &f.interface)
    }

    pub fn quote(&self, id: &DeclarationId) -> Option<&'a FunctionDeclaration> {
        let Slot::Quote { index: i, .. } = *self.positions.0.get(id)? else {
            return None;
        };
        Some(&self.quotes[i].interface)
    }

    pub(crate) fn staged_quote(&self, id: &DeclarationId) -> bool {
        matches!(
            self.positions.0.get(id),
            Some(Slot::Quote { staged: true, .. })
        )
    }

    pub fn method_definition(&self, id: &DeclarationId) -> Option<&'a FunctionDeclaration> {
        self.interface(id).or_else(|| self.quote(id))
    }

    pub fn runtime_identity(&self, id: &DeclarationId) -> Option<&'a RuntimeFunctionIdentity> {
        let Slot::Runtime(i) = *self.positions.0.get(id)? else {
            return None;
        };
        Some(&self.runtime[i])
    }

    pub(crate) fn phase(&self, id: &DeclarationId) -> Option<(FunctionPhase, bool, bool)> {
        if let Some(f) = self.method_definition(id) {
            Some((f.phase, !f.preconditions.is_empty(), f.is_unsafe))
        } else {
            self.runtime_identity(id).map(|f| {
                (
                    FunctionPhase::Runtime,
                    false,
                    f.signature.is_unsafe_callable(),
                )
            })
        }
    }
}

pub(crate) fn validate_frame<'a>(
    program: crate::contracts::CallableProgram<'a>,
    quote: bool,
    fuel: &mut usize,
) -> Result<&'a [crate::Parameter], Error> {
    program.validate_input_frame(fuel).map_err(|failure| {
        if failure.is_budget() {
            Error::budget(failure.source, failure.message(quote))
        } else {
            Error::invalid(failure.source, failure.message(quote))
        }
    })
}
