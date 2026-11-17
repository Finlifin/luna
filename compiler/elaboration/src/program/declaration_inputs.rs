//! Shared source-order input plans, independent of the eventual type constructor.
use super::*;
mod collections;
mod completion;
mod domain;
mod environment;
mod interface;
pub(super) mod preparation;
mod progress;
pub(super) mod request;
mod routing;
mod scope;
mod slots;

/// Consumers that cannot yet suspend frontend checking require ordinary typed
/// expressions. Both consumers retain the same routing and execution plan.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputChecking {
    Contextual,
    Typed,
}

pub(super) use completion::Completion;
pub(super) use environment::Environment;
pub(super) use environment::copy_term as copy_input_domain;

// Shared only within this preparation and its result consumer. Domain
// values move out of the caller's temporary telescope instead of borrowing it.
type Parameters = std::sync::Arc<[(DeclarationId, Sort, Term)]>;

pub(super) struct PreparedDeclarationInputs {
    pub arguments: Vec<n::NormalFormArgument>,
    pub plan: n::NormalFormInputPlan,
    pub formation: FormationInputs,
}

/// Typed consumers receive ordinary NIR only, with execution statements separate
/// from the declaration-order result slots and their formation evidence.
pub(super) struct CheckedDeclarationInputs {
    pub arguments: Vec<n::Expr>,
    pub statements: Vec<n::Expr>,
    pub formation: FormationInputs,
}

impl std::fmt::Display for CheckedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checked-declaration-inputs (statements")?;
        for statement in &self.statements {
            write!(f, " {statement}")?;
        }
        f.write_str(") (arguments")?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (identities")?;
        for term in &self.formation.terms {
            write!(f, " {term}")?;
        }
        f.write_str(") (deferred")?;
        for parameter in &self.formation.deferred {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (environment")?;
        for (parameter, value) in &self.formation.environment {
            write!(f, " (input {parameter} {value})")?;
        }
        f.write_str("))")
    }
}

impl PreparedDeclarationInputs {
    pub(super) fn into_typed(
        self,
        parameters: &[(DeclarationId, Sort, Term)],
        span: Span,
    ) -> Result<CheckedDeclarationInputs, Error> {
        let n::NormalFormInputPlan {
            order,
            preparations,
            bindings,
        } = self.plan;
        let mut sources = self.arguments.into_iter().map(Some).collect::<Vec<_>>();
        let mut outputs = vec![None; sources.len()];
        let mut preparations = preparations.into_iter().peekable();
        let mut statements = Vec::new();
        for (position, slot) in order.into_iter().enumerate() {
            while preparations
                .peek()
                .is_some_and(|preparation| preparation.before <= position)
            {
                statements.push(preparations.next().unwrap().expression);
            }
            let Some(n::NormalFormArgument::Value(value)) = sources[slot].take() else {
                return Err(error(span, "typed declaration input was not checked"));
            };
            let parameter = parameters[slot].0;
            let ty = value.ty.clone();
            let output = if matches!(value.kind, K::TypeValue(_)) {
                value
            } else {
                let location = value.source;
                statements.push(n::Expr {
                    source: location,
                    ty: if is_never(&ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::Let {
                        binding: bindings[slot],
                        value: Box::new(value),
                    },
                });
                n::Expr {
                    source: location,
                    ty,
                    kind: if parameters[slot].2 == p(P::Type)
                        && !self.formation.deferred.contains(&parameter)
                    {
                        K::TypeValue(self.formation.terms[slot].clone())
                    } else {
                        K::Local(bindings[slot])
                    },
                }
            };
            outputs[slot] = Some(output);
        }
        statements.extend(preparations.map(|preparation| preparation.expression));
        Ok(CheckedDeclarationInputs {
            arguments: outputs.into_iter().map(Option::unwrap).collect(),
            statements,
            formation: self.formation,
        })
    }

    pub fn diverges(&self) -> bool {
        self.arguments
            .iter()
            .any(n::NormalFormArgument::is_diverging)
            || self
                .plan
                .preparations
                .iter()
                .any(|p| is_never(&p.expression.ty))
    }
}

impl std::fmt::Display for PreparedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(prepared-declaration-inputs (arguments")?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {} (identities", self.plan)?;
        for term in &self.formation.terms {
            write!(f, " {term}")?;
        }
        f.write_str(") (deferred")?;
        for parameter in &self.formation.deferred {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (environment")?;
        for (parameter, value) in &self.formation.environment {
            write!(f, " (input {parameter} {value})")?;
        }
        f.write_str("))")
    }
}

enum RoutedDeclarationInput<'a, 'h> {
    Source(&'a Expr<'h>),
    Prepared(n::Expr),
    Collected(Vec<n::NormalFormArgument>),
}

impl std::fmt::Display for RoutedDeclarationInput<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(input) => write!(f, "(source {} {})", input.hir_id, source(input.span)),
            Self::Prepared(value) => write!(f, "(prepared {value})"),
            Self::Collected(items) => {
                f.write_str("(collected")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Source references awaiting collection checking, not another expression IR.
/// `before` records the number of fixed inputs preceding this source input.
struct PendingDeclarationCollection<'a, 'h> {
    source_index: usize,
    before: usize,
    input: &'a Expr<'h>,
    spread: bool,
}

impl std::fmt::Display for PendingDeclarationCollection<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(pending-declaration-collection (source-index {}) (before {}) (spread {}) {})",
            self.source_index,
            self.before,
            self.spread,
            source(self.input.span)
        )
    }
}

/// A caller-scoped domain can remain generic while all of this call's
/// dependencies are supplied. Pending refers to callee inputs, not rigidity.
struct DeclarationInputDomain {
    ty: Term,
    pending: bool,
}
impl std::fmt::Display for DeclarationInputDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-domain (pending {}) (type {}))",
            self.pending, self.ty
        )
    }
}

impl Checker<'_, '_> {
    pub(super) fn planned_positional_inputs(
        &mut self,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        collector: Option<usize>,
    ) -> Result<CheckedDeclarationInputs, Error> {
        preparation::Frame::positional(
            self,
            inputs,
            span,
            parameters,
            collector,
            completion::Typed,
        )?
        .drive(self)
    }

    // Inspect the original callee domain before substitution. A supplied
    // caller parameter may have the same declaration identity as another
    // callee input in a recursive call; it is not another pending dependency.
    fn declaration_domain_pending(
        &mut self,
        domain: &Term,
        parameters: &[(DeclarationId, Sort, Term)],
        completed: &[DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(completed.len() as u64)
            .ok_or_else(|| error(span, "input domain completion budget exhausted"))?;
        let completed = completed.iter().copied().collect::<BTreeSet<_>>();
        input_observation::pending_domain_inputs(
            domain,
            parameters
                .iter()
                .map(|(id, _, _)| (*id, completed.contains(id))),
            &mut self.required_steps,
            span,
        )
        .map(|pending| !pending.is_empty())
    }

    fn prepare_declaration_collection_input(
        &mut self,
        value: n::Expr,
        before: usize,
        preparations: &mut Vec<n::NormalFormPreparation>,
        known_bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let binding = self.spread_binding(span)?;
        let known = self.preserved_input_identity(
            &value,
            known_bindings
                .iter()
                .map(|(binding, term)| (*binding, term.as_ref())),
            span,
        )?;
        if let Some(known) = known {
            known_bindings.insert(binding, std::sync::Arc::new(known));
        }

        let ty = value.ty.clone();
        self.locals.insert(binding, ty.clone());
        let location = value.source;
        let diverges = is_never(&ty);
        let tuple_ty = if diverges {
            p(P::Never)
        } else {
            self.tuple_value_type(vec![ty.clone()], span)?
        };
        preparations.push(n::NormalFormPreparation {
            before,
            expression: n::Expr {
                source: location,
                ty: if diverges { p(P::Never) } else { p(P::Void) },
                kind: K::LetPattern {
                    pattern: n::Pattern::Tuple(vec![n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    }]),
                    value: Box::new(n::Expr {
                        source: location,
                        ty: tuple_ty,
                        kind: K::Tuple(vec![value]),
                    }),
                    guard: None,
                    otherwise: None,
                },
            },
        });
        Ok(n::Expr {
            source: location,
            ty,
            kind: K::Local(binding),
        })
    }

    fn record_declaration_input(
        &mut self,
        progress: &mut progress::Progress,
        slot: usize,
        value: n::NormalFormArgument,
        identity: Option<Term>,
        ty: Term,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<(), Error> {
        let known = identity
            .as_ref()
            .map(|known| self.copy_alias_type(known, span).map(std::sync::Arc::new))
            .transpose()?;
        progress.record(slot, value, identity, &mut self.required_steps, span)?;
        if let Some(known) = known {
            bindings.insert(progress.binding(slot), known);
        }
        self.locals.insert(progress.binding(slot), ty);
        Ok(())
    }

    /// Retain declaration-checked code until the VM has admitted the actual
    /// prefix. Rebinding changes local identities, not the prefix's availability.
    fn connected_normal_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        progress: &progress::Progress,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if (0..slot).any(|previous| !progress.is_prepared(previous)) {
            return Err(error(span, "normal-form default awaits preceding input"));
        }
        let mut value = self.checked_normal_default(owner, slot)?;
        constants::charge_copy(&value, &mut self.required_steps, span)?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(&value, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let mut connections = free
            .map_err(|message| error(span, message))?
            .into_iter()
            .map(|binding| (binding, binding))
            .collect::<BTreeMap<_, _>>();
        for (previous, (_, binding, _)) in self.normal_parameters[&owner][..slot].iter().enumerate()
        {
            connections.insert(*binding, progress.binding(previous));
        }
        cmtt::freshen(
            &mut value,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            span,
        )?;
        value.rewrite_types(&mut |ty| progress.instantiate(self, ty, span))?;
        Ok(value)
    }

    /// Prepare explicit inputs in source order, then omitted inputs in their
    /// declaration telescope. All reuse goes through fresh local bindings.
    pub(super) fn planned_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<PreparedDeclarationInputs, Error> {
        self.plan_declaration_inputs(
            owner,
            inputs,
            span,
            parameters,
            defaults,
            completion::Contextual,
        )
    }

    pub(super) fn planned_typed_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<CheckedDeclarationInputs, Error> {
        self.plan_declaration_inputs(owner, inputs, span, parameters, defaults, completion::Typed)
    }

    fn plan_declaration_inputs<C: Completion>(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
        completion: C,
    ) -> Result<C::Output, Error> {
        let defaults = defaults
            .map(|view| Environment::retain(view, &mut self.required_steps, span))
            .transpose()?;
        self.complete_declaration_inputs(owner, inputs, span, parameters, defaults, completion)
    }

    pub(super) fn complete_declaration_inputs<C: Completion>(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<Environment>,
        completion: C,
    ) -> Result<C::Output, Error> {
        preparation::Frame::begin(self, owner, inputs, span, parameters, defaults, completion)?
            .drive(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_dump_retains_source_slots_identities_and_escaped_values() {
        let location = n::SourceRange { start: 3, end: 9 };
        let id = DeclarationId {
            package: 1,
            local: 7,
        };
        let value = n::Expr {
            source: location,
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("quote\"\n".into())),
        };
        let inputs = PreparedDeclarationInputs {
            arguments: vec![n::NormalFormArgument::Value(value)],
            plan: n::NormalFormInputPlan {
                order: vec![0],
                bindings: vec![n::LocalId { owner: 4, index: 2 }],
                preparations: vec![],
            },
            formation: FormationInputs {
                terms: vec![p(P::U8)],
                deferred: BTreeSet::from([id]),
                environment: BTreeMap::new(),
            },
        };
        let dump = inputs.to_string();
        assert!(
            dump.starts_with("(prepared-declaration-inputs (arguments "),
            "{dump}"
        );
        assert!(dump.contains("(source 3 9)"), "{dump}");
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(order 0)"), "{dump}");
        assert!(dump.contains("(bindings (local 4 2))"), "{dump}");
        assert!(dump.contains("(identities (primitive u8))"), "{dump}");
        assert!(
            dump.contains("(deferred (declaration (package 1) (local 7)))"),
            "{dump}"
        );
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
        let checked = inputs
            .into_typed(&[(id, Sort::ComptimeValue, p(P::Str))], Span::default())
            .unwrap();
        let dump = checked.to_string();
        assert!(
            dump.starts_with("(checked-declaration-inputs (statements "),
            "{dump}"
        );
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(arguments (expr"), "{dump}");
        assert!(dump.contains("(local 4 2)"), "{dump}");
        assert!(dump.contains("(identities (primitive u8))"), "{dump}");
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
    }
}
