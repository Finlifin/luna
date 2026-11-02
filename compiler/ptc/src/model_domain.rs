//! Closed model elements, distinct from executable/source NIR value formation.
//! In particular, a payload's interpreted Type tag is not a source layout.
//! Source formulas retain normal scope/formation validation; only model
//! witnesses use this domain, and exported substitutions require source proofs.
use super::*;

pub(super) fn closed_type(term: &Term, nominal: &BTreeSet<DeclarationId>) -> bool {
    // This checks the model domain, not executable NIR formation. Interpreted
    // payload tags can denote a closed Type without exposing a source layout.
    // Keep callable shape/sort checks here as well as recursive closure: merely
    // having a constructor tag does not make a forged wrapper a valid Type.
    let valid_shape = match term {
        Term::ComptimeFunction(signature) => matches!(signature.as_ref(), Term::Function { .. }),
        Term::NormalFormFunction(signature, collector) => {
            matches!(signature.as_ref(), Term::Function { parameters, .. }
                if parameters.iter().all(|parameter| parameter.sort != Sort::Runtime)
                    && collector.is_none_or(|index| index.checked_add(1) == Some(parameters.len())))
        }
        Term::UnsafeFunction(signature) => matches!(
            signature.as_ref(),
            Term::Function { .. } | Term::ComptimeFunction(_) | Term::NormalFormFunction(..)
        ),
        Term::Closure {
            environment,
            signature,
        } => {
            // Empty closure qualification is an alias, not a distinct model element.
            !environment.is_empty()
                && matches!(
                    signature.as_ref(),
                    Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::NormalFormFunction(..)
                        | Term::UnsafeFunction(_)
                )
        }
        _ => true,
    };
    if !valid_shape {
        return false;
    }
    if let Term::Computation { effects, result } = term {
        return model_effects::canonical(effects, result, nominal);
    }
    if let Term::ErrorResult { errors, result } = term {
        // Model equality uses completed canonical values. Reject forged raw
        // assignments rather than interpreting label order or duplicate tags
        // as distinct Type identities. Source terms are normalized by rebuild.
        if errors.is_empty()
            || matches!(result.as_ref(), Term::ErrorResult { .. })
            || !closed_type(result, nominal)
        {
            return false;
        }
        // Match the constructor's stable ordering without cloning the entire
        // row or allocating another sorted collection during witness replay.
        let mut seen = std::collections::HashSet::new();
        let mut previous = None;
        for label in errors {
            if !closed_type(label, nominal) || !seen.insert(label) {
                return false;
            }
            let key = label.to_string();
            if previous.as_ref().is_some_and(|previous| previous > &key) {
                return false;
            }
            previous = Some(key);
        }
        return true;
    }
    if let Term::Nominal {
        declaration,
        arguments,
    } = term
    {
        // A nominal Type's parameters are ordinary comptime inputs, not an
        // implicit all-Type telescope. Declaration formation remains the
        // producer's responsibility; model replay checks completed arguments.
        return (arguments.is_empty() || nominal.contains(declaration))
            && arguments
                .iter()
                .all(|argument| model_applications::closed_argument(argument, nominal));
    }
    constructor(term).is_some()
        && !matches!(term, Term::Primitive(Primitive::Unit))
        && subterms(term)
            .into_iter()
            .all(|child| closed_type(child, nominal))
}
