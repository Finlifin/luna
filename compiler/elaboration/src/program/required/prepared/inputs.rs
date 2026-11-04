//! Route supplied values by capture identity before touching invocation state.
//! Value domains and transport permissions are checked by invocation admission.
use super::*;

type Capture = (n::LocalId, Term, Option<Term>);

/// A route is in supply order; each destination is a distinct pending capture.
/// Inspect only identities and availability, never clone or interpret payloads.
pub(super) fn routes<T>(
    captures: &[Capture],
    supplied: &[(n::LocalId, T)],
    span: Span,
) -> Result<Vec<usize>, Error> {
    let mut slots = BTreeMap::new();
    for (position, (binding, _, value)) in captures.iter().enumerate() {
        if slots
            .insert(*binding, (position, value.is_some()))
            .is_some()
        {
            return Err(error(span, "duplicate deferred comptime capture"));
        }
    }
    let mut assigned = BTreeSet::new();
    let mut routes = Vec::with_capacity(supplied.len());
    for (binding, _) in supplied {
        if !assigned.insert(*binding) {
            return Err(error(span, "duplicate deferred comptime input"));
        }
        match slots.get(binding) {
            Some((position, false)) => routes.push(*position),
            Some((_, true)) => {
                return Err(error(span, "deferred comptime input is already supplied"));
            }
            None => return Err(error(span, "unknown deferred comptime input")),
        }
    }
    Ok(routes)
}

/// Complete the ABI-ordered packet without freezing live graphs. An incomplete
/// packet is rejected before a VM or any source computation can be started.
pub(super) fn complete(
    captures: Vec<Capture>,
    supplied: Vec<(n::LocalId, InvocationValue)>,
    span: Span,
) -> Result<Vec<(Term, InvocationValue)>, Error> {
    let routes = routes(&captures, &supplied, span)?;
    let mut inputs: Vec<_> = captures
        .into_iter()
        .map(|(_, domain, value)| (domain, value.map(InvocationValue::Normal)))
        .collect();
    for (position, (_, value)) in routes.into_iter().zip(supplied) {
        inputs[position].1 = Some(value);
    }
    inputs.into_iter().map(|(domain, value)| {
        value.map(|value| (domain, value)).ok_or_else(|| error(span,
            "type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented"))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_preserve_capture_order_domains_and_transport_kinds() {
        let id = |owner| n::LocalId { owner, index: 0 };
        let captures = vec![
            (id(20), p(P::Bool), None),
            (id(10), p(P::Type), Some(p(P::U8))),
            (id(30), p(P::Str), None),
        ];
        let text: Arc<str> = "shared".into();
        let values = vec![
            (id(30), InvocationValue::Live(Value::String(text.clone()))),
            (id(20), InvocationValue::Normal(Term::Bool(true))),
        ];
        let output = complete(captures, values, Span::default()).unwrap();
        assert!(
            matches!(&output[0], (ty, InvocationValue::Normal(Term::Bool(true))) if *ty == p(P::Bool))
        );
        assert!(
            matches!(&output[1], (ty, InvocationValue::Normal(value)) if *ty == p(P::Type) && *value == p(P::U8))
        );
        assert!(
            matches!(&output[2], (ty, InvocationValue::Live(Value::String(value))) if *ty == p(P::Str) && Arc::ptr_eq(value, &text))
        );
    }

    #[test]
    fn malformed_capture_layout_is_rejected_before_payloads_are_used() {
        let id = n::LocalId { owner: 1, index: 0 };
        let captures = vec![
            (id, p(P::Bool), None),
            (id, p(P::Bool), Some(Term::Bool(true))),
        ];
        let result = complete(captures, vec![], Span::default());
        assert!(
            matches!(result, Err(error) if error.message.contains("duplicate deferred comptime capture"))
        );
    }
}
