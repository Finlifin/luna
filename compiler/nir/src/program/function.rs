//! Checked function declarations and their separately owned bodies.
use super::*;

/// Canonical package/owner namespaces reserved for compiler-synthesized
/// implicit runtime environment parameters backing local `Term::Runtime` dependencies.
pub const IMPLICIT_RUNTIME_ENV_PACKAGE: u64 = u64::MAX - 9;

pub const IMPLICIT_RUNTIME_ENV_OWNER: u32 = u32::MAX - 1;

/// Local identities that require additional ABI slots, in canonical order.
pub fn implicit_runtime_env_ids(
    runtime_domains: &std::collections::BTreeMap<crate::RuntimeId, Term>,
    globals: &std::collections::BTreeMap<crate::RuntimeId, Expr>,
    arguments: &[Term],
    bound: impl IntoIterator<Item = crate::RuntimeId>,
) -> Result<std::collections::BTreeSet<crate::RuntimeId>, &'static str> {
    let mut ids = std::collections::BTreeSet::new();
    for argument in arguments {
        ids.extend(argument.runtime_dependencies());
    }
    for id in bound {
        ids.remove(&id);
    }
    ids.retain(|id| {
        !globals.contains_key(id) && matches!(runtime_domains.get(id), Some(Term::Lift(_)))
    });
    if ids.iter().any(|id| id.instance != 0) {
        return Err("implicit runtime environment parameter requires instance 0 identity");
    }
    Ok(ids)
}

/// Collect implicit runtime environment parameters required by local identities
/// inside the instance arguments, after the declaration's explicit slots.
pub fn implicit_runtime_env_parameters(
    runtime_domains: &std::collections::BTreeMap<crate::RuntimeId, Term>,
    global_runtime_deliveries: &std::collections::BTreeMap<crate::RuntimeId, Expr>,
    arguments: &[Term],
    existing: &[RuntimeEnvParameter],
) -> Result<Vec<RuntimeEnvParameter>, &'static str> {
    let ids = implicit_runtime_env_ids(
        runtime_domains,
        global_runtime_deliveries,
        arguments,
        existing.iter().filter_map(|env| env.runtime_id),
    )?;
    let mut extra = Vec::with_capacity(ids.len());
    for id in ids {
        let Term::Lift(inner) = &runtime_domains[&id] else {
            unreachable!("checked lift domain")
        };
        extra.push(RuntimeEnvParameter {
            declaration: DeclarationId {
                package: IMPLICIT_RUNTIME_ENV_PACKAGE,
                local: id.local,
            },
            runtime_id: Some(id),
            binding: LocalId {
                owner: IMPLICIT_RUNTIME_ENV_OWNER,
                index: id.local,
            },
            ty: (**inner).clone(),
        });
    }
    Ok(extra)
}

/// Runtime parameter carrying the concrete address/handle of a `quote`
/// dependency bound into a runtime function's caller environment.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeEnvParameter {
    pub declaration: DeclarationId,
    pub runtime_id: Option<crate::RuntimeId>,
    pub binding: LocalId,
    pub ty: Term,
}

impl fmt::Display for RuntimeEnvParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(env-param (declaration {})", self.declaration)?;
        if let Some(id) = self.runtime_id {
            write!(f, " (runtime-id {id})")?;
        }
        write!(f, " (binding {}) (type {}))", self.binding, self.ty)
    }
}

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
    pub result_type_binder: Option<DeclarationId>,
    pub result_guarantees: Vec<TypePredicate>,
    pub entry_snapshots: Vec<EntrySnapshot>,
    pub capabilities: Vec<TraitRequirement>,
    pub requirements: Vec<TypePredicate>,
    pub type_parameters: Vec<DeclarationId>,
    pub declaration: DeclarationId,
    pub name: String,
    pub signature: Term,
    pub parameters: Vec<LocalId>,
    pub env_parameters: Vec<RuntimeEnvParameter>,
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
        let substitution = ParameterSubstitution::for_fragments(parameters, arguments)?;
        self.with_parameter_substitution(&substitution)
    }

    fn with_parameter_substitution(
        &self,
        substitution: &ParameterSubstitution<'_, '_>,
    ) -> Result<Self, crate::TypeError> {
        Ok(Self {
            source: self.source,
            phase: self.phase,
            inline: self.inline,
            pure: self.pure,
            is_unsafe: self.is_unsafe,
            result_binding: self.result_binding,
            result_type_binder: self.result_type_binder,
            entry_snapshots: self.entry_snapshots.clone(),
            type_parameters: self.type_parameters.clone(),
            declaration: self.declaration,
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            env_parameters: self
                .env_parameters
                .iter()
                .map(|env| {
                    let substituted = substitution.term(&Term::Rigid {
                        declaration: env.declaration,
                        sort: crate::Sort::Runtime,
                    })?;
                    let (declaration, runtime_id) = match substituted {
                        Term::Runtime(id) => (env.declaration, Some(id)),
                        Term::Rigid {
                            declaration: next,
                            sort: crate::Sort::Runtime,
                        } => (next, env.runtime_id),
                        _ => (env.declaration, env.runtime_id),
                    };
                    Ok(RuntimeEnvParameter {
                        declaration,
                        runtime_id,
                        binding: env.binding,
                        ty: substitution.term(&env.ty)?,
                    })
                })
                .collect::<Result<_, crate::TypeError>>()?,
            signature: substitution.term(&self.signature)?,
            requirements: self
                .requirements
                .iter()
                .map(|p| p.try_map_types(&mut |ty| substitution.term(ty)))
                .collect::<Result<_, _>>()?,
            result_guarantees: self
                .result_guarantees
                .iter()
                .map(|p| p.try_map_types(&mut |ty| substitution.term(ty)))
                .collect::<Result<_, _>>()?,
            capabilities: self
                .capabilities
                .iter()
                .map(|p| p.try_map_types(&mut |ty| substitution.term(ty)))
                .collect::<Result<_, _>>()?,
            preconditions: self
                .preconditions
                .iter()
                .map(|e| e.with_parameter_substitution(substitution))
                .collect::<Result<_, _>>()?,
            postconditions: self
                .postconditions
                .iter()
                .map(|e| e.with_parameter_substitution(substitution))
                .collect::<Result<_, _>>()?,
            defaults: self
                .defaults
                .iter()
                .map(|(index, e)| {
                    e.with_parameter_substitution(substitution)
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
    pub runtime_deliveries: std::collections::BTreeMap<crate::RuntimeId, Expr>,
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
        let substitution = ParameterSubstitution::for_fragments(parameters, arguments)?;
        Ok(Self {
            interface: self.interface.with_parameter_substitution(&substitution)?,
            body: self.body.with_parameter_substitution(&substitution)?,
            runtime_deliveries: self
                .runtime_deliveries
                .iter()
                .map(|(id, expr)| Ok((*id, expr.with_parameter_substitution(&substitution)?)))
                .collect::<Result<_, crate::TypeError>>()?,
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
        let frames = [crate::function_parameters::ordinary_frame(
            &self.signature,
            fuel,
        )?];
        let substitution = crate::DeclarationSubstitution::bounded_domains(
            &self.type_parameters,
            domains,
            arguments,
            crate::DeclarationInputScope::ClosedInBody(&frames),
            fuel,
        )?;
        self.specialize_body(&substitution, fuel)
    }

    /// Materialize a concrete function after checking its complete declaration
    /// input telescope against the receiving world's layout and callable queries.
    pub fn instantiate_admitted_parameters<D: std::borrow::Borrow<Term>>(
        &self,
        domains: &[D],
        arguments: &[Term],
        fuel: &mut usize,
        queries: &mut crate::value_admission::DomainQueries<'_, '_>,
    ) -> Result<Self, crate::NominalAdmissionError> {
        let frames = [crate::function_parameters::ordinary_frame(
            &self.signature,
            fuel,
        )?];
        let inputs = crate::admit_declaration_inputs(
            &self.type_parameters,
            domains,
            arguments,
            crate::DeclarationInputScope::ClosedInBody(&frames),
            0,
            fuel,
            queries,
        )
        .map_err(|failure| failure.reason)?;
        Ok(self.specialize_body(&inputs.into_substitution(), fuel)?)
    }

    fn specialize_body(
        &self,
        scoped: &crate::DeclarationSubstitution<'_>,
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        scoped.charge_function_substitution(self, fuel)?;
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
                    result_type_binder: self.result_type_binder,
                    entry_snapshots: self.entry_snapshots.clone(),
                    type_parameters: self.type_parameters.clone(),
                    declaration: self.declaration,
                    name: self.name.clone(),
                    parameters: self.parameters.clone(),
                    env_parameters: self
                        .env_parameters
                        .iter()
                        .map(|env| {
                            let substituted = scoped.instantiate(&Term::Rigid {
                                declaration: env.declaration,
                                sort: crate::Sort::Runtime,
                            })?;
                            let (declaration, runtime_id) = match substituted {
                                Term::Runtime(id) => (env.declaration, Some(id)),
                                Term::Rigid {
                                    declaration: next,
                                    sort: crate::Sort::Runtime,
                                } => (next, env.runtime_id),
                                _ => (env.declaration, env.runtime_id),
                            };
                            Ok(RuntimeEnvParameter {
                                declaration,
                                runtime_id,
                                binding: env.binding,
                                ty: scoped.instantiate(&env.ty)?,
                            })
                        })
                        .collect::<Result<_, crate::TypeError>>()?,
                    signature: scoped.instantiate_closed_signature(&self.signature)?,
                    requirements: self
                        .requirements
                        .iter()
                        .map(|p| scoped.instantiate_predicate(p))
                        .collect::<Result<_, _>>()?,
                    result_guarantees: self
                        .result_guarantees
                        .iter()
                        .map(|p| scoped.instantiate_predicate(p))
                        .collect::<Result<_, _>>()?,
                    capabilities: self
                        .capabilities
                        .iter()
                        .map(|capability| scoped.instantiate_requirement(capability))
                        .collect::<Result<_, _>>()?,
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
                runtime_deliveries: self
                    .runtime_deliveries
                    .iter()
                    .map(|(id, value)| expression(value).map(|value| (*id, value)))
                    .collect::<Result<_, _>>()?,
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
        if !self.env_parameters.is_empty() {
            f.write_str(") (env-parameters")?;
            for env in &self.env_parameters {
                write!(f, " {env}")?;
            }
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
        f.write_str(") (result-guarantees")?;
        if let Some(binder) = self.result_type_binder {
            write!(f, " {binder}")?;
        }
        for guarantee in &self.result_guarantees {
            write!(f, " {guarantee}")?;
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
        if !self.runtime_deliveries.is_empty() {
            f.write_str(" (runtime-deliveries")?;
            for (runtime, expr) in &self.runtime_deliveries {
                write!(f, " (delivery {runtime} {expr})")?;
            }
            f.write_str(")")?;
        }
        write!(f, " (body {}))", self.body)
    }
}
