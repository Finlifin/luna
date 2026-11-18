//! Checked function declarations and their separately owned bodies.
use super::*;

/// Checked declaration metadata, independent of the executable body.
/// The signature is a telescope, not the complete public supply protocol.
/// Owning this interface grants neither invocation nor phase authority.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDeclaration {
    /// Location of the declaration or the expression that generated it.
    pub source: SourceRange,
    pub phase: FunctionPhase,
    /// Source inlining intent; not mandatory expansion or a purity promise.
    pub inline: bool,
    /// Trusted source promise; never authorizes normal-form unfolding.
    pub pure: bool,
    /// Caller must establish an explicit unsafe boundary; independent of phase/purity.
    pub is_unsafe: bool,
    /// Checked program entry predicates; never sent to the type solver.
    pub preconditions: Vec<Expr>,
    pub postconditions: Vec<Expr>,
    pub result_binding: Option<LocalId>,
    pub entry_snapshots: Vec<EntrySnapshot>,
    pub capabilities: Vec<TraitRequirement>,
    pub requirements: Vec<TypePredicate>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub name: String,
    pub signature: Term,
    pub parameters: Vec<LocalId>,
    pub defaults: Vec<(usize, Expr)>,
}
impl FunctionDeclaration {
    /// Value interface of this declaration; the stored signature remains its
    /// parameter/result telescope for instantiation and ABI consumers.
    pub fn callable_type(&self) -> Term {
        self.callable_type_view().into_owned()
    }

    pub fn callable_type_view(&self) -> std::borrow::Cow<'_, Term> {
        if self.phase == FunctionPhase::Comptime || self.is_unsafe {
            std::borrow::Cow::Owned(
                crate::callable_source::Formation::from(self).qualify(self.signature.clone()),
            )
        } else {
            std::borrow::Cow::Borrowed(&self.signature)
        }
    }
    /// Substitute checked declaration fields without acquiring or copying a
    /// body. Binder identities and local input slots are preserved.
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Term::Tuple(Vec::new()).instantiate_parameters(parameters, arguments)?;
        Ok(Self {
            source: self.source,
            phase: self.phase,
            inline: self.inline,
            pure: self.pure,
            is_unsafe: self.is_unsafe,
            result_binding: self.result_binding,
            entry_snapshots: self.entry_snapshots.clone(),
            type_parameters: self.type_parameters.clone(),
            declaration: self.declaration,
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            signature: self
                .signature
                .instantiate_parameters(parameters, arguments)?,
            requirements: self
                .requirements
                .iter()
                .map(|p| p.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            capabilities: self
                .capabilities
                .iter()
                .map(|p| p.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            preconditions: self
                .preconditions
                .iter()
                .map(|e| e.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            postconditions: self
                .postconditions
                .iter()
                .map(|e| e.instantiate_parameters(parameters, arguments))
                .collect::<Result<_, _>>()?,
            defaults: self
                .defaults
                .iter()
                .map(|(index, e)| {
                    e.instantiate_parameters(parameters, arguments)
                        .map(|e| (*index, e))
                })
                .collect::<Result<_, _>>()?,
        })
    }
}

/// A checked expression in the projected frame of an open definition.
/// The owner identifies its lexical source. The region has no callable identity.
#[derive(Clone)]
pub struct DefinitionRegion {
    pub owner: DeclarationId,
    pub phase: FunctionPhase,
    pub signature: Term,
    pub parameters: Vec<LocalId>,
    pub type_parameters: Vec<DeclarationId>,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
    pub body: Expr,
}

impl fmt::Display for DefinitionRegion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(definition-region (owner {}) (phase {}) (signature {}) (parameters",
            self.owner, self.phase, self.signature
        )?;
        for binding in &self.parameters {
            write!(f, " {binding}")?;
        }
        f.write_str(") (type-parameters")?;
        for binder in &self.type_parameters {
            write!(f, " {binder}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (capabilities")?;
        for requirement in &self.capabilities {
            write!(f, " {requirement}")?;
        }
        write!(f, ") (body {}))", self.body)
    }
}

/// A fully checked function. Incomplete source bodies belong to the source
/// checker and cannot be represented by an empty or placeholder expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub interface: FunctionDeclaration,
    pub body: Expr,
}

// Existing field projections remain borrowed views of the single interface;
// signature/contract-only services can accept FunctionDeclaration directly.
impl std::ops::Deref for Function {
    type Target = FunctionDeclaration;
    fn deref(&self) -> &Self::Target {
        &self.interface
    }
}
impl std::ops::DerefMut for Function {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.interface
    }
}

impl Function {
    /// Specialize the declaration and its body under the same substitution.
    pub fn instantiate_parameters(
        &self,
        parameters: &[crate::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        Ok(Self {
            interface: self
                .interface
                .instantiate_parameters(parameters, arguments)?,
            body: self.body.instantiate_parameters(parameters, arguments)?,
        })
    }

    /// Specialize a declaration using completed domains while retaining its
    /// ordinary input frame in contracts, defaults and body annotations.
    /// Owns the complete expanded-copy preflight. Callers still admit values;
    /// structural substitution does not establish formation or resource rights.
    pub fn instantiate_declaration_parameters(
        &self,
        domains: &[Term],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        let closed = crate::DeclarationSubstitution::bounded(
            &self.type_parameters,
            domains,
            arguments,
            &[],
            fuel,
        )?;
        // Charge metadata, contracts, defaults and shared code before any
        // template copy. The scoped rewriter below consumes this preflight.
        closed.charge_function_substitution(self, fuel)?;
        let frames = match self.signature.callable_signature() {
            Term::Function { parameters, .. } => vec![parameters.iter().map(|p| p.sort).collect()],
            _ => vec![],
        };
        let scoped = crate::DeclarationSubstitution::bounded(
            &self.type_parameters,
            domains,
            arguments,
            &frames,
            fuel,
        )?;
        let specialized = (|| -> Result<Self, crate::TypeError> {
            let expression = |value: &Expr| {
                let mut value = value.clone();
                value.rewrite_types(&mut |ty| scoped.instantiate(ty))?;
                Ok::<_, crate::TypeError>(value)
            };
            Ok(Self {
                interface: FunctionDeclaration {
                    source: self.source,
                    phase: self.phase,
                    inline: self.inline,
                    pure: self.pure,
                    is_unsafe: self.is_unsafe,
                    result_binding: self.result_binding,
                    entry_snapshots: self.entry_snapshots.clone(),
                    type_parameters: self.type_parameters.clone(),
                    declaration: self.declaration,
                    name: self.name.clone(),
                    parameters: self.parameters.clone(),
                    signature: closed.instantiate(&self.signature)?,
                    requirements: self
                        .requirements
                        .iter()
                        .map(|p| scoped.instantiate_predicate(p))
                        .collect::<Result<_, _>>()?,
                    capabilities: self
                        .capabilities
                        .iter()
                        .map(|capability| {
                            let TypePredicate::Trait(value) = scoped
                                .instantiate_predicate(&TypePredicate::Trait(capability.clone()))?
                            else {
                                unreachable!()
                            };
                            Ok(value)
                        })
                        .collect::<Result<_, crate::TypeError>>()?,
                    preconditions: self
                        .preconditions
                        .iter()
                        .map(expression)
                        .collect::<Result<_, _>>()?,
                    postconditions: self
                        .postconditions
                        .iter()
                        .map(expression)
                        .collect::<Result<_, _>>()?,
                    defaults: self
                        .defaults
                        .iter()
                        .map(|(index, value)| expression(value).map(|value| (*index, value)))
                        .collect::<Result<_, _>>()?,
                },
                body: expression(&self.body)?,
            })
        })();
        specialized.map_err(crate::type_computations::Error::Scope)
    }

    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Self, crate::TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| crate::DeclarationParameter {
                declaration,
                sort: crate::Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters(&parameters, arguments)
    }
}

impl FunctionDeclaration {
    fn fmt_fields(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (name ", self.declaration)?;
        string(f, &self.name)?;
        write!(
            f,
            ") {} (phase {}) (inline {}) (pure {}) (unsafe {}) (capabilities",
            self.source, self.phase, self.inline, self.pure, self.is_unsafe
        )?;
        for requirement in &self.capabilities {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (signature {}) (parameters", self.signature)?;
        for p in &self.parameters {
            write!(f, " {p}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (program-requires")?;
        for condition in &self.preconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in &self.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(binding) = self.result_binding {
            write!(f, " {binding}")?;
        }
        f.write_str(") (program-ensures")?;
        for condition in &self.postconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (defaults")?;
        for (parameter, value) in &self.defaults {
            write!(f, " (default (parameter {parameter}) {value})")?;
        }
        f.write_str(")")
    }
}
impl fmt::Display for FunctionDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(function-declaration ")?;
        self.fmt_fields(f)?;
        f.write_str(")")
    }
}
impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(function ")?;
        self.interface.fmt_fields(f)?;
        write!(f, " (body {}))", self.body)
    }
}
