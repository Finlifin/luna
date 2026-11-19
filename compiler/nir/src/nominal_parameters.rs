//! Nominal declaration binders. Layout substitution preserves value-indexed
//! identities; domain admission and trait/program obligations remain separate.
use crate::{DeclarationId, DeclarationParameter, Primitive, Sort, Term, TypeError};
use std::borrow::{Borrow, Cow};

mod telescope;
use telescope::Binders;

#[cfg(test)]
mod owned_substitution_tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 19, local }
    }

    #[test]
    fn owned_substitution_keeps_completed_domains_and_caller_scope() {
        let (owned, dump) = {
            let parameters = [id(1), id(2)];
            let domains = [
                Term::Primitive(Primitive::Type),
                Term::Rigid {
                    declaration: id(1),
                    sort: Sort::Type,
                },
            ];
            let arguments = [
                Term::Primitive(Primitive::Type),
                Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                },
            ];
            let frames = [vec![Sort::Type]];
            let prepared = DeclarationSubstitution::bounded(
                &parameters,
                &domains,
                &arguments,
                &frames,
                &mut 1000,
            )
            .unwrap();
            let dump = prepared.to_string();
            (prepared.into_owned(&mut 1000).unwrap(), dump)
        };
        assert_eq!(owned.to_string(), dump);
        let template = Term::NormalForm {
            head: id(30),
            sort: Sort::Type,
            arguments: vec![Term::Rigid {
                declaration: id(2),
                sort: Sort::ComptimeValue,
            }],
        };
        assert_eq!(
            owned.instantiate_bounded(&template, &mut 1000).unwrap(),
            Term::NormalForm {
                head: id(30),
                sort: Sort::Type,
                arguments: vec![Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type
                }],
            }
        );
    }

    #[test]
    fn owning_a_substitution_charges_borrowed_payloads_before_copying() {
        let parameters = [id(1)];
        let domains = [Term::Primitive(Primitive::Type)];
        let arguments = [Term::Tuple(vec![Term::Primitive(Primitive::Bool); 256])];
        let prepared =
            DeclarationSubstitution::bounded(&parameters, &domains, &arguments, &[], &mut 10_000)
                .unwrap();
        assert!(matches!(
            prepared.into_owned(&mut 16),
            Err(crate::type_computations::Error::Budget)
        ));
        let prepared =
            DeclarationSubstitution::bounded(&parameters, &domains, &arguments, &[], &mut 10_000)
                .unwrap();
        let mut fuel = 10_000;
        let owned = prepared.into_owned(&mut fuel).unwrap();
        assert!(10_000 - fuel >= 256);
        assert_eq!(
            owned
                .instantiate_bounded(
                    &Term::Rigid {
                        declaration: id(1),
                        sort: Sort::Type,
                    },
                    &mut 10_000
                )
                .unwrap(),
            arguments[0]
        );
    }
}

pub use crate::value_admission::Error as AdmissionError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAdmissionStage {
    Domain,
    Value,
}
impl std::fmt::Display for InputAdmissionStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Domain => "domain",
            Self::Value => "value",
        })
    }
}

/// The slot belongs to the declaration, even when its domain was delivered
/// out of order. Whole-telescope failures have no individual consumer.
#[derive(Debug)]
pub struct DeclarationAdmissionFailure {
    pub slot: Option<(usize, InputAdmissionStage)>,
    pub reason: AdmissionError,
}
impl From<AdmissionError> for DeclarationAdmissionFailure {
    fn from(reason: AdmissionError) -> Self {
        Self { slot: None, reason }
    }
}
impl std::fmt::Display for DeclarationAdmissionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(declaration-admission-failure")?;
        if let Some((slot, stage)) = self.slot {
            write!(f, " (slot {slot}) (stage {stage})")?;
        }
        write!(f, " (reason {:?}))", self.reason.to_string())
    }
}

pub fn binders(
    parameters: &[DeclarationId],
    domains: &[Term],
) -> Result<Vec<DeclarationParameter>, TypeError> {
    Ok(Binders::new(parameters, domains)?.ordered)
}

/// Admission of completed value indices at the concrete layout boundary.
/// Symbolic declarations need their own domain evidence and are not guessed here.
pub fn validate_completed(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), AdmissionError> {
    validate_completed_in(parameters, domains, arguments, fuel, None)
}

pub fn validate_completed_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    validate_completed_with_layouts_in(
        parameters,
        domains,
        arguments,
        fuel,
        |_, _, _| Err("nominal value admission requires declaration layout evidence".into()),
        target,
    )
}

/// Admission with authoritative declaration-order payload domains. The callback
/// must validate the nominal instance and variant, or a registered Tuple head
/// for tuple value indices. A value's own type tag is never sufficient evidence
/// of its layout. All work shares the caller's fuel.
pub fn validate_completed_with_layouts(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
) -> Result<(), AdmissionError> {
    validate_completed_with_layouts_in(parameters, domains, arguments, fuel, layout, None)
}

pub fn validate_completed_with_layouts_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    validate_with_evidence_in(
        parameters,
        domains,
        arguments,
        fuel,
        layout,
        |_, _, _| false,
        target,
    )
}

/// Symbolic value leaves require independent declaration-domain evidence.
pub fn validate_with_evidence(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<(), AdmissionError> {
    validate_with_evidence_in(parameters, domains, arguments, fuel, layout, declared, None)
}

pub fn validate_with_evidence_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    admit_with_evidence_in(
        parameters,
        domains,
        arguments,
        fuel,
        &mut layout,
        &mut declared,
        target,
    )
    .map(|_| ())
}

/// Admit immutable collections using independently registered element domains.
/// The callback must charge lookup and domain copying to the shared fuel.
/// Neither a frozen value's tag nor an opaque head's shape grants this evidence.
pub fn validate_with_collections_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    collection: impl FnMut(&Term, &mut usize) -> Result<Option<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    admit_with_collections_in(
        parameters, domains, arguments, fuel, layout, declared, collection, target,
    )
    .map(|_| ())
}

/// Failure of a copied expression specialization. Copy limits retain their
/// reason for execution consumers; scope failures never become budget failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpressionInstantiationError {
    Copy(crate::copy_budget::CopyBudgetExhausted),
    Scope(TypeError),
}
impl std::fmt::Display for ExpressionInstantiationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Copy(reason) => write!(f, "expression specialization copy limit: {reason:?}"),
            Self::Scope(reason) => write!(f, "expression specialization scope: {reason}"),
        }
    }
}
impl From<ExpressionInstantiationError> for crate::type_computations::Error {
    fn from(error: ExpressionInstantiationError) -> Self {
        match error {
            ExpressionInstantiationError::Copy(_) => Self::Budget,
            ExpressionInstantiationError::Scope(reason) => Self::Scope(reason),
        }
    }
}

/// Structural declaration substitution in an explicit caller scope. This checks
/// scopes and sorts, including a dependent domain becoming Type. It does not
/// validate scalar ranges, aggregate payloads, requirements or capabilities;
/// those remain formation/admission obligations at the owning boundary.
pub struct DeclarationSubstitution<'a> {
    parameters: Binders,
    arguments: Cow<'a, [Term]>,
    frames: Cow<'a, [Vec<Sort>]>,
}
impl std::fmt::Display for DeclarationSubstitution<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(declaration-substitution (inputs")?;
        for (parameter, argument) in self.parameters.ordered.iter().zip(self.arguments.iter()) {
            write!(f, " (input {parameter} {argument})")?;
        }
        f.write_str(") ")?;
        display_frames(f, &self.frames)?;
        f.write_str(")")
    }
}
fn display_frames(f: &mut std::fmt::Formatter<'_>, frames: &[Vec<Sort>]) -> std::fmt::Result {
    f.write_str("(frames")?;
    for frame in frames {
        f.write_str(" (frame")?;
        for sort in frame {
            write!(f, " {sort}")?;
        }
        f.write_str(")")?;
    }
    f.write_str(")")
}

/// Check the complete observed substitution at handoff without taking ownership
/// of the values. This establishes scope and effective sorts only.
pub fn check_observed_inputs<'a>(
    observed: impl Iterator<Item = (DeclarationId, &'a Term, &'a Term)>,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<(), crate::type_computations::Error> {
    let mut ids = Vec::new();
    let mut domains = Vec::new();
    let mut values = Vec::new();
    for (id, domain, value) in observed {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(crate::type_computations::Error::Budget)?;
        ids.push(id);
        domains.push(domain);
        values.push(value);
    }
    Binders::bounded_substitution(&ids, &domains, &values, frames, fuel)?;
    Ok(())
}

/// Specialize one domain using only its observed declaration inputs. Follow
/// dependencies in declaration domains to check a replacement's effective sort.
/// Replacement values belong to the caller and are never followed as templates.
pub fn instantiate_observed_domain<'a>(
    domain: &Term,
    mut observed: impl FnMut(DeclarationId) -> Option<(&'a Term, &'a Term)>,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Term, crate::type_computations::Error> {
    use crate::type_computations::rigid_declarations_bounded;
    use std::collections::BTreeSet;

    let mut pending = BTreeSet::new();
    rigid_declarations_bounded(domain, fuel, &mut pending)?;
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    let mut domains = Vec::new();
    let mut values = Vec::new();
    while let Some(id) = pending.pop_first() {
        if seen.insert(id)
            && let Some((template, value)) = observed(id)
        {
            rigid_declarations_bounded(template, fuel, &mut pending)?;
            ids.push(id);
            domains.push(template);
            values.push(value);
        }
    }
    let binders = Binders::bounded_substitution(&ids, &domains, &values, frames, fuel)?;
    binders.instantiate_bounded(domain, &values, frames, fuel)
}

impl<'a> DeclarationSubstitution<'a> {
    pub fn new(
        ids: &[DeclarationId],
        domains: &[Term],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
    ) -> Result<Self, TypeError> {
        Self::from_parameters(
            Binders::new(ids, domains)?,
            domains,
            arguments,
            DeclarationInputScope::Caller(frames),
        )
    }
    fn from_parameters<D: Borrow<Term>>(
        parameters: Binders,
        domains: &[D],
        arguments: &'a [Term],
        scope: DeclarationInputScope<'a>,
    ) -> Result<Self, TypeError> {
        parameters.check_substitution(domains, arguments, scope.input())?;
        Ok(Self {
            parameters,
            arguments: Cow::Borrowed(arguments),
            frames: Cow::Borrowed(scope.template()),
        })
    }
    pub fn bounded(
        ids: &[DeclarationId],
        domains: &[Term],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        Self::bounded_domains(
            ids,
            domains,
            arguments,
            DeclarationInputScope::Caller(frames),
            fuel,
        )
    }

    pub fn bounded_domains<D: Borrow<Term>>(
        ids: &[DeclarationId],
        domains: &[D],
        arguments: &'a [Term],
        scope: DeclarationInputScope<'a>,
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        let parameters =
            Binders::bounded_substitution(ids, domains, arguments, scope.input(), fuel)?;
        Ok(Self {
            parameters,
            arguments: Cow::Borrowed(arguments),
            frames: Cow::Borrowed(scope.template()),
        })
    }
    /// Check and take ownership of the caller's existing buffers. Unlike
    /// retaining a borrowed substitution, this does not copy any terms.
    pub fn bounded_owned<D: Borrow<Term>>(
        ids: &[DeclarationId],
        domains: &[D],
        arguments: Vec<Term>,
        frames: Vec<Vec<Sort>>,
        fuel: &mut usize,
    ) -> Result<DeclarationSubstitution<'static>, crate::type_computations::Error> {
        let parameters = Binders::bounded_substitution(ids, domains, &arguments, &frames, fuel)?;
        Ok(DeclarationSubstitution {
            parameters,
            arguments: Cow::Owned(arguments),
            frames: Cow::Owned(frames),
        })
    }

    pub fn arguments(&self) -> &[Term] {
        &self.arguments
    }

    pub fn into_arguments(self) -> Cow<'a, [Term]> {
        self.arguments
    }

    /// Retain a structurally checked substitution beyond its input buffers.
    /// This owns syntax and scope only, never invocation storage or proof authority.
    /// All borrowed payloads are charged before the first owned copy is made.
    pub fn into_owned(
        self,
        fuel: &mut usize,
    ) -> Result<DeclarationSubstitution<'static>, crate::type_computations::Error> {
        use crate::type_computations::{Error, charge_term};
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if let Cow::Borrowed(arguments) = &self.arguments {
            for argument in *arguments {
                charge_term(argument, fuel)?;
            }
        }
        if let Cow::Borrowed(frames) = &self.frames {
            *fuel = fuel.checked_sub(frames.len()).ok_or(Error::Budget)?;
            for frame in *frames {
                *fuel = fuel.checked_sub(frame.len()).ok_or(Error::Budget)?;
            }
        }
        Ok(DeclarationSubstitution {
            parameters: self.parameters,
            arguments: Cow::Owned(self.arguments.into_owned()),
            frames: Cow::Owned(self.frames.into_owned()),
        })
    }

    pub fn instantiate(&self, term: &Term) -> Result<Term, TypeError> {
        self.parameters
            .instantiate(term, &self.arguments, &self.frames)
    }
    /// A complete source function signature binds its own ordinary parameters.
    pub(crate) fn instantiate_closed_signature(&self, signature: &Term) -> Result<Term, TypeError> {
        self.parameters.instantiate(signature, &self.arguments, &[])
    }

    /// Project the complete signature outside the ordinary frame it introduces.
    pub fn instantiate_closed_signature_bounded(
        &self,
        signature: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        self.parameters
            .instantiate_bounded(signature, &self.arguments, &[], fuel)
    }

    /// Validate a template's body, defaults and contracts without constructing
    /// executable syntax. The receiver supplies the ordinary parameter frame.
    pub fn check_retained_body(
        &self,
        function: &crate::program::Function,
        fuel: &mut usize,
    ) -> Result<(), crate::type_computations::Error> {
        crate::copy_budget::check_retained_body(function, self, fuel)
    }

    pub(crate) fn instantiate_requirement(
        &self,
        requirement: &crate::program::TraitRequirement,
    ) -> Result<crate::program::TraitRequirement, TypeError> {
        requirement.try_map_types(&mut |ty| self.instantiate(ty))
    }
    pub fn instantiate_predicate(
        &self,
        predicate: &crate::program::TypePredicate,
    ) -> Result<crate::program::TypePredicate, TypeError> {
        predicate.try_map_types(&mut |ty| self.instantiate(ty))
    }
    fn charge_template(
        &self,
        root: crate::copy_budget::Node<'_>,
        fuel: &mut usize,
    ) -> Result<(), crate::copy_budget::CopyBudgetExhausted> {
        crate::copy_budget::charge_projected(root, fuel, &|id| {
            self.parameters.replacement(id, self.arguments.as_ref())
        })
    }

    pub(crate) fn charge_function_substitution(
        &self,
        function: &crate::program::Function,
        fuel: &mut usize,
    ) -> Result<(), crate::type_computations::Error> {
        self.charge_template(crate::copy_budget::Node::Function(function), fuel)
            .map_err(|_| crate::type_computations::Error::Budget)
    }

    /// Copy and specialize a checked expression only after bounding the entire
    /// expanded tree, including shared closure bodies and hidden type operands.
    /// The source is immutable; a failed rewrite never publishes a partial tree.
    pub fn instantiate_expression_bounded(
        &self,
        expression: &crate::program::Expr,
        fuel: &mut usize,
    ) -> Result<crate::program::Expr, ExpressionInstantiationError> {
        self.parameters.instantiate_expression(
            expression,
            self.arguments.as_ref(),
            &self.frames,
            fuel,
        )
    }

    pub fn instantiate_predicate_bounded(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        self.charge_template(crate::copy_budget::Node::Predicate(predicate), fuel)
            .map_err(|_| crate::type_computations::Error::Budget)?;
        self.instantiate_predicate(predicate)
            .map_err(crate::type_computations::Error::Scope)
    }
    pub fn instantiate_interface_bounded(
        &self,
        interface: &crate::program::InterfaceInstance,
        fuel: &mut usize,
    ) -> Result<crate::program::InterfaceInstance, crate::type_computations::Error> {
        interface.try_map_types(&mut |ty| self.instantiate_bounded(ty, fuel))
    }
    pub fn instantiate_requirement_bounded(
        &self,
        requirement: &crate::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<crate::program::TraitRequirement, crate::type_computations::Error> {
        requirement.try_map_types(&mut |ty| self.instantiate_bounded(ty, fuel))
    }
    pub fn instantiate_bounded(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        self.parameters
            .instantiate_bounded(term, &self.arguments, &self.frames, fuel)
    }
}

/// Completed declaration inputs with checked domains. Only this evidence permits
/// a definition-side value binder to become Type when its actual domain is Type.
/// Strict mixed-sort substitution remains available for fixed-sort frames.
pub struct AdmittedArguments<'a, T = Term> {
    parameters: Binders,
    arguments: &'a [T],
    scope: DeclarationInputScope<'a>,
}
impl<T: Borrow<Term>> std::fmt::Display for AdmittedArguments<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(admitted-arguments")?;
        for (parameter, argument) in self.parameters.ordered.iter().zip(self.arguments) {
            write!(f, " (input {parameter} {})", argument.borrow())?;
        }
        f.write_str(" ")?;
        write!(f, "{}", self.scope)?;
        f.write_str(")")
    }
}
impl<'a> AdmittedArguments<'a> {
    /// Keep the checked binder index and the template's ordinary telescope for
    /// code in this activation. Rewriting code does not grant admission again.
    pub fn into_substitution(self) -> DeclarationSubstitution<'a> {
        DeclarationSubstitution {
            parameters: self.parameters,
            arguments: Cow::Borrowed(self.arguments),
            frames: Cow::Borrowed(self.scope.template()),
        }
    }
}

impl<T: Borrow<Term>> AdmittedArguments<'_, T> {
    pub fn instantiate(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        self.parameters
            .instantiate_bounded(term, self.arguments, self.scope.template(), fuel)
    }
    /// A whole callable signature introduces its own ordinary telescope. A
    /// requirement in that body instead inherits the template frames.
    pub fn instantiate_in_input_scope(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        self.parameters
            .instantiate_bounded(term, self.arguments, self.scope.input(), fuel)
    }
    pub(crate) fn instantiate_expression(
        &self,
        expression: &crate::program::Expr,
        fuel: &mut usize,
    ) -> Result<crate::program::Expr, ExpressionInstantiationError> {
        self.parameters.instantiate_expression(
            expression,
            self.arguments,
            self.scope.template(),
            fuel,
        )
    }
    pub fn instantiate_requirement(
        &self,
        requirement: &crate::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<crate::program::TraitRequirement, crate::type_computations::Error> {
        requirement.try_map_types(&mut |ty| self.instantiate(ty, fuel))
    }
    pub fn instantiate_predicate(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        self.predicate(predicate, fuel, 0)
    }
    fn predicate(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        use crate::program::TypePredicate as P;
        use crate::type_computations::Error;
        if depth >= 128 {
            return Err(Error::Budget);
        }
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        Ok(match predicate {
            P::Bool(value) => P::Bool(*value),
            P::Trait(requirement) => P::Trait(self.instantiate_requirement(requirement, fuel)?),
            P::Equal(a, b) => P::Equal(self.instantiate(a, fuel)?, self.instantiate(b, fuel)?),
            P::Kind(ty, kind) => P::Kind(self.instantiate(ty, fuel)?, *kind),
            P::Not(inner) => P::Not(Box::new(self.predicate(inner, fuel, depth + 1)?)),
            P::And(parts) | P::Or(parts) => {
                let parts = parts
                    .iter()
                    .map(|part| self.predicate(part, fuel, depth + 1))
                    .collect::<Result<_, _>>()?;
                if matches!(predicate, P::And(_)) {
                    P::And(parts)
                } else {
                    P::Or(parts)
                }
            }
        })
    }
}

pub fn admit_with_evidence_in<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    target: Option<crate::target::TargetData>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    admit_with_collections_in(
        parameters,
        domains,
        arguments,
        fuel,
        layout,
        declared,
        |_, _| Ok(None),
        target,
    )
}

/// Admit a completed telescope and retain its checked substitution for formation
/// predicates. Collection metadata and symbolic declarations are independent
/// evidence sources; callbacks must charge lookup work to the shared budget.
pub fn admit_with_collections_in<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    mut collection: impl FnMut(&Term, &mut usize) -> Result<Option<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    admit_with_queries_in(
        parameters,
        domains,
        arguments,
        fuel,
        &mut crate::value_admission::DomainQueries {
            layout: &mut layout,
            declared: &mut |value, domain, fuel| Ok(declared(value, domain, fuel)),
            collection: &mut collection,
            callable: &mut |_, _, _| Ok(None),
            target,
        },
    )
}

/// Admit a telescope using receiving-world evidence, preserving failed queries
/// as errors instead of treating them as missing declaration evidence.
pub fn admit_with_queries_in<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
    queries: &mut crate::value_admission::DomainQueries<'_, '_>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    admit_in_scope(parameters, domains, arguments, &[], fuel, queries)
}

/// Admit arguments in the enclosing callable telescope. Keep that scope with
/// the evidence so subsequent formation checks use the same binder depths.
pub fn admit_in_scope<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    frames: &'a [Vec<Sort>],
    fuel: &mut usize,
    queries: &mut crate::value_admission::DomainQueries<'_, '_>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    admit_in_order(
        parameters,
        domains,
        arguments,
        DeclarationInputScope::Caller(frames),
        0,
        fuel,
        queries,
    )
    .map_err(|failure| failure.reason)
}

/// Admit a declaration-ordered projection of invocation storage. The
/// receiving world checks every value; the projection borrows their storage.
pub fn admit_projected_in_scope<'a, 'value>(
    parameters: &[DeclarationId],
    domains: &[&Term],
    arguments: &'a [&'value Term],
    frames: &'a [Vec<Sort>],
    fuel: &mut usize,
    queries: &mut crate::value_admission::DomainQueries<'_, '_>,
) -> Result<AdmittedArguments<'a, &'value Term>, AdmissionError> {
    admit_in_order(
        parameters,
        domains,
        arguments,
        DeclarationInputScope::Caller(frames),
        0,
        fuel,
        queries,
    )
    .map_err(|failure| failure.reason)
}

/// A queried callable may live under an outer binder. At a source call, its
/// declaration inputs instead arrive closed, even when the contract template
/// mentions ordinary parameters of the callable being formed.
#[derive(Clone, Copy)]
pub enum DeclarationInputScope<'a> {
    Caller(&'a [Vec<Sort>]),
    ClosedInBody(&'a [Vec<Sort>]),
}
impl<'a> DeclarationInputScope<'a> {
    fn input(self) -> &'a [Vec<Sort>] {
        match self {
            Self::Caller(frames) => frames,
            Self::ClosedInBody(_) => &[],
        }
    }
    fn template(self) -> &'a [Vec<Sort>] {
        match self {
            Self::Caller(frames) | Self::ClosedInBody(frames) => frames,
        }
    }
}
impl std::fmt::Display for DeclarationInputScope<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(declaration-input-scope (caller ")?;
        display_frames(f, self.input())?;
        write!(f, ") (template ")?;
        display_frames(f, self.template())?;
        f.write_str("))")
    }
}

pub fn admit_in_order<'a, D: Borrow<Term>, T: Borrow<Term>>(
    parameters: &[DeclarationId],
    domains: &[D],
    arguments: &'a [T],
    scope: DeclarationInputScope<'a>,
    first: usize,
    fuel: &mut usize,
    queries: &mut crate::value_admission::DomainQueries<'_, '_>,
) -> Result<AdmittedArguments<'a, T>, DeclarationAdmissionFailure> {
    *fuel = fuel
        .checked_sub(parameters.len())
        .ok_or(AdmissionError::Budget)?;
    for domain in domains {
        crate::type_computations::charge_term(domain.borrow(), fuel)
            .map_err(AdmissionError::from)?;
    }
    let parameters =
        Binders::new(parameters, domains).map_err(|e| AdmissionError::from(e.to_string()))?;
    if parameters.ordered.len() != arguments.len() {
        return Err(AdmissionError::from("declaration argument count mismatch").into());
    }
    for argument in arguments {
        let argument = argument.borrow();
        crate::type_computations::charge_term(argument, fuel).map_err(AdmissionError::from)?;
        argument
            .check_scope(scope.input())
            .map_err(|e| AdmissionError::from(e.to_string()))?;
    }
    parameters.admit(domains, arguments, scope.input(), first, fuel, queries)?;
    Ok(AdmittedArguments {
        parameters,
        arguments,
        scope,
    })
}

impl crate::callable_source::Formation<'_> {
    /// Admit a call's owned declaration arguments once. Its signature introduces
    /// the ordinary binder frame; body and contract templates inherit that frame.
    pub fn admit_instance<D: Borrow<Term>>(
        self,
        domains: &[D],
        arguments: Vec<Term>,
        fuel: &mut usize,
        queries: &mut crate::value_admission::DomainQueries<'_, '_>,
    ) -> Result<(Term, DeclarationSubstitution<'static>), DeclarationAdmissionFailure> {
        let frames = vec![
            crate::function_parameters::ordinary_frame(self.signature, fuel)
                .map_err(AdmissionError::from)?,
        ];
        let admitted = admit_in_order(
            self.parameters,
            domains,
            &arguments,
            DeclarationInputScope::ClosedInBody(&frames),
            0,
            fuel,
            queries,
        )?;
        let signature = admitted
            .instantiate_in_input_scope(self.signature, fuel)
            .map_err(AdmissionError::from)?;
        let substitution = DeclarationSubstitution {
            parameters: admitted.parameters,
            arguments: Cow::Owned(arguments),
            frames: Cow::Owned(frames),
        };
        Ok((self.qualify(signature), substitution))
    }
}

/// The caller has already checked scope and charged the input tree. Symbolic
/// leaves require independent declaration-domain evidence supplied by `declared`.
pub(crate) fn validate_symbolic_value_in(
    argument: &Term,
    domain: &Term,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> Result<bool, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    crate::value_admission::validate(
        argument,
        domain,
        frames,
        fuel,
        &mut crate::value_admission::DomainQueries {
            layout: &mut layout,
            declared: &mut declared,
            collection: &mut |_, _| Ok(None),
            callable: &mut |_, _, _| Ok(None),
            target,
        },
    )
}
