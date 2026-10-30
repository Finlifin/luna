//! Stable contextual expression-code interfaces shared by elaboration consumers.
//!
//! An interface carries runtime identities and explicit replaceable ports. It
//! is not an ordinary callable result type, nor a runtime code object. Keeping
//! these maps beside the result prevents method contracts from erasing the
//! distinction between fixed dependencies and supplied context ports.
use crate::{Sort, Term, program::LocalId, type_computations};
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
    /// Ordinary data inputs whose values bind dependencies in later domains.
    /// Keys are parameter positions; declaration identities are local binders,
    /// not inference variables or additional clause arguments.
    pub input_binders: BTreeMap<usize, crate::DeclarationParameter>,
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
        f.write_str(") (input-binders")?;
        for (slot, binder) in &self.input_binders {
            write!(
                f,
                " (binder (parameter {slot}) (declaration {}) (sort {:?}))",
                binder.declaration, binder.sort
            )?;
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
        self.output.charge_output(fuel)?;
        self.validate_input_binders(fuel)
    }

    /// A telescope entry can depend only on earlier ordinary inputs. Clause
    /// domains are outside this telescope; code values themselves are not Terms.
    fn validate_input_binders(&self, fuel: &mut usize) -> Result<(), type_computations::Error> {
        use type_computations::Error;
        let mut seen = std::collections::BTreeSet::new();
        for (slot, binder) in &self.input_binders {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            let Some((_, CodeInput::Data(domain))) = self.inputs.get(*slot) else {
                return Err(Error::Structure(
                    "dependent binder must belong to a data input".into(),
                ));
            };
            let sort = if domain == &Term::Primitive(crate::Primitive::Type) {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            if binder.sort != sort || !seen.insert(binder.declaration) {
                return Err(Error::Structure(
                    "duplicate or incorrectly sorted dependent input binder".into(),
                ));
            }
            for term in self.clause_domains.iter().chain(
                self.inputs[..=*slot]
                    .iter()
                    .flat_map(|(_, input)| input.type_terms()),
            ) {
                type_computations::charge_term(term, fuel)?;
                if term.mentions_rigid(binder.declaration) {
                    return Err(Error::Structure(
                        "dependent input is used before its binding".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Apply one already evaluated ordinary input without evaluating any other
    /// argument. Value-domain admission remains the caller's responsibility.
    /// The original signature is retained on failure, including its binders.
    pub fn specialize_input(
        &self,
        slot: usize,
        value: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, type_computations::Error> {
        use type_computations::Error;
        self.charge(fuel)?;
        let binder = self
            .input_binders
            .get(&slot)
            .ok_or_else(|| Error::Structure("input has no dependent binder".into()))?;
        type_computations::charge_term(value, fuel)?;
        value.check_scope(frames).map_err(Error::Scope)?;
        if value.sort() != binder.sort {
            return Err(Error::Structure(
                "dependent input value has the wrong sort".into(),
            ));
        }
        let parameters = std::slice::from_ref(binder);
        let arguments = std::slice::from_ref(value);
        let mut completed = self.try_map_types(|ty| {
            crate::copy_budget::charge_type_substitution(ty, parameters, arguments, fuel)
                .map_err(|_| Error::Budget)?;
            ty.instantiate_parameters_in_scope(parameters, arguments, frames)
                .map_err(Error::Scope)
        })?;
        completed.input_binders.remove(&slot);
        completed.charge(fuel)?;
        Ok(completed)
    }

    /// Alpha-align ordinary input binders before comparing two contextual method
    /// contracts. This changes declaration references, never runtime LocalIds.
    pub fn align_input_binders(
        &self,
        reference: &Self,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<std::borrow::Cow<'_, Self>, type_computations::Error>
    where
        O: Clone,
    {
        use type_computations::Error;
        if self.input_binders.is_empty() && reference.input_binders.is_empty() {
            return Ok(std::borrow::Cow::Borrowed(self));
        }
        self.charge(fuel)?;
        reference.charge(fuel)?;
        if self.input_binders.len() != reference.input_binders.len() {
            return Err(Error::Structure(
                "dependent input binding positions differ".into(),
            ));
        }
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for ((slot, binder), (other_slot, other)) in
            self.input_binders.iter().zip(&reference.input_binders)
        {
            if slot != other_slot || binder.sort != other.sort {
                return Err(Error::Structure(
                    "dependent input binding positions or sorts differ".into(),
                ));
            }
            parameters.push(binder.clone());
            arguments.push(Term::Rigid {
                declaration: other.declaration,
                sort: other.sort,
            });
        }
        let mut aligned = self.try_map_types(|ty| {
            crate::copy_budget::charge_type_substitution(ty, &parameters, &arguments, fuel)
                .map_err(|_| Error::Budget)?;
            ty.instantiate_parameters_in_scope(&parameters, &arguments, frames)
                .map_err(Error::Scope)
        })?;
        aligned.input_binders = reference.input_binders.clone();
        Ok(std::borrow::Cow::Owned(aligned))
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
            input_binders: self.input_binders.clone(),
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

/// Interface members with code inputs or results retain their contextual signature;
/// ordinary callable consumers must request an ordinary type explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MethodSignature {
    Callable(Term),
    Generator(CodeFunctionSignature),
    PatternGenerator(PatternFunctionSignature),
    DataConsumer(DataFunctionSignature),
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
            Self::DataConsumer(signature) => signature.fmt(f),
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
            Self::DataConsumer(signature) => Some(&signature.clause_domains),
        }
    }

    pub fn ordinary(&self) -> Result<&Term, crate::TypeError> {
        match self {
            Self::Callable(ty) => Ok(ty),
            Self::Generator(_) | Self::PatternGenerator(_) | Self::DataConsumer(_) => {
                Err(crate::TypeError::ContextualCodeNotCallable)
            }
        }
    }
    pub fn is_comptime_callable(&self) -> bool {
        match self {
            Self::Callable(ty) => ty.is_comptime_callable(),
            Self::Generator(_) | Self::PatternGenerator(_) | Self::DataConsumer(_) => true,
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
            Self::DataConsumer(signature) => {
                Self::DataConsumer(signature.try_map_types(transform)?)
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
        let (ordinary, code, pattern, data) = match self {
            Self::Callable(ty) => (Some(ty), None, None, None),
            Self::Generator(signature) => (None, Some(signature), None, None),
            Self::PatternGenerator(signature) => (None, None, Some(signature), None),
            Self::DataConsumer(signature) => (None, None, None, Some(signature)),
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
            .chain(
                data.into_iter()
                    .flat_map(|signature| signature.type_terms()),
            )
    }
}
