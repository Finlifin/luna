//! Stable contextual expression-code interfaces shared by elaboration consumers.
//!
//! An interface carries runtime identities and explicit replaceable ports. It
//! is not an ordinary callable result type, nor a runtime code object. Keeping
//! these maps beside the result prevents method contracts from erasing the
//! distinction between fixed dependencies and supplied context ports.
use crate::{Term, program::LocalId, type_computations};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionCodeInterface {
    /// Call-site expected result without rebinding a quote's lexical inputs.
    pub infer_context: bool,
    pub context: BTreeMap<LocalId, Term>,
    pub ports: BTreeMap<String, LocalId>,
    pub defaults: BTreeMap<LocalId, LocalId>,
    pub result: Term,
}

impl fmt::Display for ExpressionCodeInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(code-interface (infer-context {}) (context",
            self.infer_context
        )?;
        for (binding, ty) in &self.context {
            write!(f, " (binding {binding} {ty})")?;
        }
        f.write_str(") (ports")?;
        for (name, binding) in &self.ports {
            write!(f, " (port {name:?} {binding})")?;
        }
        f.write_str(") (defaults")?;
        for (port, actual) in &self.defaults {
            write!(f, " (connect {port} {actual})")?;
        }
        write!(f, ") (result {}))", self.result)
    }
}

impl ExpressionCodeInterface {
    /// Charge every owned type and metadata entry before copying an interface.
    /// Port spelling is user-controlled and must be included in the budget.
    pub fn charge(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        let work = self
            .context
            .len()
            .checked_add(self.defaults.len())
            .and_then(|work| work.checked_add(1))
            .ok_or(type_computations::Error::Budget)?;
        *fuel = fuel
            .checked_sub(work)
            .ok_or(type_computations::Error::Budget)?;
        for name in self.ports.keys() {
            let work = name
                .len()
                .checked_add(1)
                .ok_or(type_computations::Error::Budget)?;
            *fuel = fuel
                .checked_sub(work)
                .ok_or(type_computations::Error::Budget)?;
        }
        for ty in self.context.values().chain(std::iter::once(&self.result)) {
            type_computations::charge_term(ty, fuel)?;
        }
        Ok(())
    }

    /// Transform all types without mutating the original on failure. The
    /// caller supplies its substitution/scope policy and charges copy work.
    /// Binding identities, exposed names and default connections are retained;
    /// type substitution alone never makes a fixed dependency replaceable.
    pub fn try_map_types<E>(
        &self,
        mut transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        let result = transform(&self.result)?;
        let context = self
            .context
            .iter()
            .map(|(binding, ty)| Ok((*binding, transform(ty)?)))
            .collect::<Result<_, E>>()?;
        Ok(Self {
            infer_context: self.infer_context,
            context,
            ports: self.ports.clone(),
            defaults: self.defaults.clone(),
            result,
        })
    }
}

/// Pattern-code subject and named success outputs, independent of local binder IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternCodeInterface {
    pub subject: Term,
    pub outputs: BTreeMap<String, Term>,
}
impl fmt::Display for PatternCodeInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pattern-interface (subject {}) (outputs", self.subject)?;
        for (name, ty) in &self.outputs {
            write!(f, " (output {name:?} {ty})")?;
        }
        f.write_str("))")
    }
}
impl PatternCodeInterface {
    pub fn charge(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        type_computations::charge_term(&self.subject, fuel)?;
        for (name, ty) in &self.outputs {
            *fuel = fuel
                .checked_sub(name.len().saturating_add(1))
                .ok_or(type_computations::Error::Budget)?;
            type_computations::charge_term(ty, fuel)?;
        }
        Ok(())
    }
    pub fn try_map_types<E>(
        &self,
        mut transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            subject: transform(&self.subject)?,
            outputs: self
                .outputs
                .iter()
                .map(|(name, ty)| Ok((name.clone(), transform(ty)?)))
                .collect::<Result<_, E>>()?,
        })
    }
}

/// A generator argument is either completed comptime data or contextual code.
/// In particular, code arguments must not be represented by their runtime
/// result types when comparing method contracts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeInput {
    Data(Term),
    Code(ExpressionCodeInterface),
    Pattern(PatternCodeInterface),
}

/// A contextual call preserves each input category independently of its result.
/// The default output is expression code for compatibility with its producers;
/// pattern producers and data consumers use the same argument telescope.
/// Clause binders and invocation routing remain on the enclosing declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeFunctionSignature<O = ExpressionCodeInterface> {
    /// Method-local clause domains in declaration order; Type domains bind
    /// types, all other domains bind comptime values.
    pub clause_domains: Vec<Term>,
    pub inputs: Vec<(LocalId, CodeInput)>,
    pub output: O,
}

impl fmt::Display for CodeInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(ty) => write!(f, "(data {ty})"),
            Self::Code(interface) => write!(f, "(code {interface})"),
            Self::Pattern(interface) => write!(f, "(pattern {interface})"),
        }
    }
}

/// Operations shared by code and ordinary-data results. They preserve the
/// output category; changing a signature result requires an explicit new type.
pub trait SignatureOutput: Sized + fmt::Display {
    const DUMP_KIND: &'static str;
    fn charge_output(&self, fuel: &mut usize) -> Result<(), type_computations::Error>;
    fn try_map_output<E>(&self, transform: impl FnMut(&Term) -> Result<Term, E>)
    -> Result<Self, E>;
    fn output_terms(&self) -> impl Iterator<Item = &Term>;
}

impl SignatureOutput for ExpressionCodeInterface {
    const DUMP_KIND: &'static str = "code-function-signature";
    fn charge_output(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        self.charge(fuel)
    }
    fn try_map_output<E>(
        &self,
        transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        self.try_map_types(transform)
    }
    fn output_terms(&self) -> impl Iterator<Item = &Term> {
        self.context.values().chain(std::iter::once(&self.result))
    }
}

impl SignatureOutput for PatternCodeInterface {
    const DUMP_KIND: &'static str = "pattern-function-signature";
    fn charge_output(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        self.charge(fuel)
    }
    fn try_map_output<E>(
        &self,
        transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        self.try_map_types(transform)
    }
    fn output_terms(&self) -> impl Iterator<Item = &Term> {
        std::iter::once(&self.subject).chain(self.outputs.values())
    }
}

impl SignatureOutput for Term {
    const DUMP_KIND: &'static str = "data-consumer-signature";
    fn charge_output(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        type_computations::charge_term(self, fuel)
    }
    fn try_map_output<E>(
        &self,
        mut transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        transform(self)
    }
    fn output_terms(&self) -> impl Iterator<Item = &Term> {
        std::iter::once(self)
    }
}

impl<O: SignatureOutput> fmt::Display for CodeFunctionSignature<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({} (clauses", O::DUMP_KIND)?;
        for domain in &self.clause_domains {
            write!(f, " (domain {domain})")?;
        }
        f.write_str(") (inputs")?;
        for (binding, input) in &self.inputs {
            write!(f, " (input {binding} {input})")?;
        }
        write!(f, ") (output {}))", self.output)
    }
}

impl<O: SignatureOutput> CodeFunctionSignature<O> {
    /// Visit every semantic domain without treating code as its runtime result.
    pub fn type_terms(&self) -> impl Iterator<Item = &Term> {
        self.clause_domains
            .iter()
            .chain(self.inputs.iter().flat_map(|(_, input)| input.type_terms()))
            .chain(self.output.output_terms())
    }

    pub fn charge(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        for domain in &self.clause_domains {
            type_computations::charge_term(domain, fuel)?;
        }
        *fuel = fuel
            .checked_sub(self.inputs.len())
            .ok_or(type_computations::Error::Budget)?;
        for (_, input) in &self.inputs {
            match input {
                CodeInput::Data(ty) => type_computations::charge_term(ty, fuel)?,
                CodeInput::Code(interface) => interface.charge(fuel)?,
                CodeInput::Pattern(interface) => interface.charge(fuel)?,
            }
        }
        self.output.charge_output(fuel)
    }

    /// Apply a declaration's completed substitution to every input and result.
    /// The data/code distinction and all contextual identities are preserved.
    pub fn try_map_types<E>(
        &self,
        mut transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        let inputs = self
            .inputs
            .iter()
            .map(|(binding, input)| {
                let input = match input {
                    CodeInput::Data(ty) => CodeInput::Data(transform(ty)?),
                    CodeInput::Pattern(interface) => {
                        CodeInput::Pattern(interface.try_map_types(&mut transform)?)
                    }
                    CodeInput::Code(interface) => {
                        CodeInput::Code(interface.try_map_types(&mut transform)?)
                    }
                };
                Ok((*binding, input))
            })
            .collect::<Result<_, E>>()?;
        Ok(Self {
            clause_domains: self
                .clause_domains
                .iter()
                .map(&mut transform)
                .collect::<Result<_, E>>()?,
            inputs,
            output: self.output.try_map_output(transform)?,
        })
    }
}

/// Pattern-producing contextual calls retain named success outputs.
pub type PatternFunctionSignature = CodeFunctionSignature<PatternCodeInterface>;

/// A comptime call may consume code while returning ordinary data, including
/// Type. This signature is not an ordinary callable Term: its inputs still
/// carry contextual interfaces and require code-aware call admission.
pub type DataFunctionSignature = CodeFunctionSignature<Term>;

/// Interface members with code results retain their contextual signature;
/// ordinary callable consumers must request an ordinary type explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MethodSignature {
    Callable(Term),
    Generator(CodeFunctionSignature),
    PatternGenerator(PatternFunctionSignature),
}
impl From<Term> for MethodSignature {
    fn from(ty: Term) -> Self {
        Self::Callable(ty)
    }
}
impl fmt::Display for MethodSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Callable(ty) => ty.fmt(f),
            Self::Generator(signature) => signature.fmt(f),
            Self::PatternGenerator(signature) => signature.fmt(f),
        }
    }
}
impl PartialEq<Term> for MethodSignature {
    fn eq(&self, other: &Term) -> bool {
        matches!(self,Self::Callable(ty) if ty == other)
    }
}
impl MethodSignature {
    pub fn clause_domains(&self) -> Option<&[Term]> {
        match self {
            Self::Callable(_) => None,
            Self::Generator(signature) => Some(&signature.clause_domains),
            Self::PatternGenerator(signature) => Some(&signature.clause_domains),
        }
    }

    pub fn ordinary(&self) -> Result<&Term, crate::TypeError> {
        match self {
            Self::Callable(ty) => Ok(ty),
            Self::Generator(_) | Self::PatternGenerator(_) => {
                Err(crate::TypeError::ContextualCodeNotCallable)
            }
        }
    }
    pub fn is_comptime_callable(&self) -> bool {
        match self {
            Self::Callable(ty) => ty.is_comptime_callable(),
            Self::Generator(_) | Self::PatternGenerator(_) => true,
        }
    }
    pub fn callable_signature(&self) -> Result<&Term, crate::TypeError> {
        Ok(self.ordinary()?.callable_signature())
    }
    pub fn try_map_types<E>(
        &self,
        mut transform: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        Ok(match self {
            Self::Callable(ty) => Self::Callable(transform(ty)?),
            Self::Generator(signature) => Self::Generator(signature.try_map_types(transform)?),
            Self::PatternGenerator(signature) => {
                Self::PatternGenerator(signature.try_map_types(transform)?)
            }
        })
    }
    pub fn instantiate_types(
        &self,
        parameters: &[crate::DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        self.try_map_types(|ty| ty.instantiate_types(parameters, arguments))
    }
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        self.try_map_types(|ty| ty.instantiate_parameters(parameters, arguments))
    }
    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
        scope: &[Vec<crate::Sort>],
    ) -> Result<Self, crate::TypeError> {
        self.try_map_types(|ty| ty.instantiate_parameters_in_scope(parameters, arguments, scope))
    }
}

impl CodeInput {
    pub fn type_terms(&self) -> impl Iterator<Item = &Term> {
        let (data, code, pattern) = match self {
            Self::Data(ty) => (Some(ty), None, None),
            Self::Code(interface) => (None, Some(interface), None),
            Self::Pattern(interface) => (None, None, Some(interface)),
        };
        data.into_iter()
            .chain(code.into_iter().flat_map(|interface| {
                interface
                    .context
                    .values()
                    .chain(std::iter::once(&interface.result))
            }))
            .chain(pattern.into_iter().flat_map(|interface| {
                std::iter::once(&interface.subject).chain(interface.outputs.values())
            }))
    }
}
impl MethodSignature {
    /// Visit semantic type dependencies without erasing signature categories.
    pub fn type_terms(&self) -> impl Iterator<Item = &Term> {
        let (ordinary, code, pattern) = match self {
            Self::Callable(ty) => (Some(ty), None, None),
            Self::Generator(signature) => (None, Some(signature), None),
            Self::PatternGenerator(signature) => (None, None, Some(signature)),
        };
        ordinary
            .into_iter()
            .chain(
                code.into_iter()
                    .flat_map(|signature| signature.type_terms()),
            )
            .chain(
                pattern
                    .into_iter()
                    .flat_map(|signature| signature.type_terms()),
            )
    }
}
