//! Callable input routing, independent of source nodes and parameter domains.
//!
//! Entries describe the caller's supply interface. Routes retain source indices;
//! they neither check expressions nor execute defaults or collection protocols.
use std::{collections::HashMap, fmt, hash::Hash};

#[derive(Clone, Debug)]
pub(super) struct Entry<Name> {
    pub name: Option<Name>,
    pub optional: bool,
    pub implicit: bool,
    pub variadic: bool,
    pub lambda: bool,
    pub post_target: bool,
}

impl<Name> Entry<Name> {
    pub fn anchored() -> Self {
        Self {
            name: None,
            optional: false,
            implicit: false,
            variadic: false,
            lambda: false,
            post_target: true,
        }
    }

    fn skipped(&self) -> bool {
        self.optional || self.implicit
    }
}

#[derive(Clone, Debug)]
pub(super) enum Input<Name> {
    Positional,
    Named(Name),
    PostLambda,
    Unsupported,
}

#[derive(Debug)]
pub(super) struct Route {
    pub parameter: usize,
    pub sources: Vec<usize>,
    pub collected: bool,
    pub lambda: bool,
}

#[derive(Debug)]
pub(super) struct Plan {
    pub routes: Vec<Route>,
    pub assigned: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FailureKind {
    RepeatedPostLambda,
    AmbiguousPostLambda,
    MissingPostTarget,
    UnknownName,
    Duplicate,
    AppendToComplete,
    TooMany,
    MultipleCollections,
    Unsupported,
    MissingImplicit,
    MissingAnchored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Failure {
    pub input: Option<usize>,
    pub kind: FailureKind,
}

impl FailureKind {
    fn at(self, input: usize) -> Failure {
        Failure {
            input: Some(input),
            kind: self,
        }
    }

    fn call(self) -> Failure {
        Failure {
            input: None,
            kind: self,
        }
    }
}

pub(super) fn post_lambda_target<Name>(
    entries: &[Entry<Name>],
    inputs: &[Input<Name>],
) -> Result<Option<usize>, Failure> {
    let supplied = inputs
        .iter()
        .filter(|input| matches!(input, Input::PostLambda))
        .count();
    if supplied == 0 {
        return Ok(None);
    }
    if supplied != 1 {
        return Err(FailureKind::RepeatedPostLambda.call());
    }
    let mut marked = entries.iter().enumerate().filter(|(_, entry)| entry.lambda);
    if let Some((index, _)) = marked.next() {
        if marked.next().is_some() {
            return Err(FailureKind::AmbiguousPostLambda.call());
        }
        return Ok(Some(index));
    }
    entries
        .iter()
        .rposition(|entry| entry.post_target && !entry.implicit)
        .map(Some)
        .ok_or_else(|| FailureKind::MissingPostTarget.call())
}

pub(super) fn route<Name: Eq + Hash>(
    entries: &[Entry<Name>],
    inputs: &[Input<Name>],
) -> Result<Plan, Failure> {
    let count = entries.len();
    let post_target = post_lambda_target(entries, inputs)?;
    let named: HashMap<_, _> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.name.as_ref().map(|name| (name, index)))
        .collect();
    // Reserve complete inputs before routing positions, without changing their
    // evaluation order. A reservation is not an executed or checked argument.
    let mut directly_supplied = vec![false; count];
    for (position, input) in inputs.iter().enumerate() {
        if let Input::Named(name) = input {
            let index = named
                .get(name)
                .copied()
                .ok_or_else(|| FailureKind::UnknownName.at(position))?;
            if std::mem::replace(&mut directly_supplied[index], true) {
                return Err(FailureKind::Duplicate.at(position));
            }
        }
    }
    let mut assigned = vec![false; count];
    let mut slots = vec![None; count];
    let mut routes: Vec<Route> = Vec::new();
    let mut positional = 0;
    let mut remaining = inputs
        .iter()
        .filter(|input| matches!(input, Input::Positional))
        .count();
    let mut anchored_suffix = vec![0; count + 1];
    let mut variadic_suffix = vec![0; count + 1];
    for index in (0..count).rev() {
        let entry = &entries[index];
        variadic_suffix[index] =
            variadic_suffix[index + 1] + usize::from(entry.variadic && !directly_supplied[index]);
        anchored_suffix[index] = anchored_suffix[index + 1]
            + usize::from(!entry.skipped() && !entry.variadic && post_target != Some(index));
    }
    for (position, input) in inputs.iter().enumerate() {
        let (parameter, collected) = match input {
            Input::Positional => {
                while positional < count
                    && (post_target == Some(positional)
                        || entries[positional].skipped()
                        || (entries[positional].variadic
                            && (directly_supplied[positional]
                                || remaining <= anchored_suffix[positional + 1])))
                {
                    positional += 1;
                }
                if positional == count {
                    return Err(
                        if entries
                            .iter()
                            .enumerate()
                            .any(|(index, entry)| entry.variadic && directly_supplied[index])
                        {
                            FailureKind::AppendToComplete.call()
                        } else {
                            FailureKind::TooMany.call()
                        },
                    );
                }
                let index = positional;
                let collected = entries[index].variadic;
                if collected && variadic_suffix[index + 1] != 0 {
                    return Err(FailureKind::MultipleCollections.call());
                }
                if !collected {
                    positional += 1;
                }
                remaining -= 1;
                (index, collected)
            }
            Input::Named(name) => (named[name], false),
            Input::PostLambda => (post_target.expect("post target checked"), false),
            Input::Unsupported => return Err(FailureKind::Unsupported.call()),
        };
        if let Some(slot) = slots[parameter] {
            let previous: &mut Route = &mut routes[slot];
            if !collected || !previous.collected {
                return Err(FailureKind::Duplicate.at(position));
            }
            previous.sources.push(position);
        } else {
            slots[parameter] = Some(routes.len());
            routes.push(Route {
                parameter,
                sources: vec![position],
                collected,
                lambda: entries[parameter].lambda,
            });
        }
        assigned[parameter] = true;
    }
    for (index, entry) in entries.iter().enumerate() {
        if !assigned[index] && entry.implicit {
            return Err(FailureKind::MissingImplicit.call());
        } else if !assigned[index] && entry.variadic {
            routes.push(Route {
                parameter: index,
                sources: Vec::new(),
                collected: true,
                lambda: false,
            });
            assigned[index] = true;
        } else if !assigned[index] && !entry.skipped() {
            return Err(FailureKind::MissingAnchored.call());
        }
    }
    Ok(Plan { routes, assigned })
}

impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RepeatedPostLambda => "post do parameter supplied more than once",
            Self::AmbiguousPostLambda => "ambiguous post do lambda parameter",
            Self::MissingPostTarget => "post do has no anchored parameter target",
            Self::UnknownName => "no named parameter entry",
            Self::Duplicate => "parameter supplied more than once",
            Self::AppendToComplete => "parameter supplied more than once: positional inputs cannot append to a directly supplied collection",
            Self::TooMany => "argument count mismatch: too many positional inputs",
            Self::MultipleCollections => "multiple variadic positional collections require a routing protocol not yet implemented; supply complete named packages",
            Self::Unsupported => "expanded and implicit argument routing is not implemented",
            Self::MissingImplicit => "implicit parameter requires explicit named supply; automatic supply protocol is not yet implemented",
            Self::MissingAnchored => "argument count mismatch: missing anchored parameter",
        })
    }
}

impl<Name: fmt::Display> fmt::Display for Entry<Name> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-entry (name ")?;
        match &self.name {
            Some(name) => write!(f, "{:?}", name.to_string())?,
            None => f.write_str("none")?,
        }
        write!(
            f,
            ") (optional {}) (implicit {}) (variadic {}) (lambda {}) (post-target {}))",
            self.optional, self.implicit, self.variadic, self.lambda, self.post_target
        )
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(input-route (parameter {}) (sources", self.parameter)?;
        for source in &self.sources {
            write!(f, " {source}")?;
        }
        write!(
            f,
            ") (collected {}) (lambda {}))",
            self.collected, self.lambda
        )
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-routing (routes")?;
        for route in &self.routes {
            write!(f, " {route}")?;
        }
        f.write_str(") (assigned")?;
        for (index, assigned) in self.assigned.iter().enumerate() {
            if *assigned {
                write!(f, " {index}")?;
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collected(name: &str) -> Entry<&str> {
        Entry {
            name: Some(name),
            variadic: true,
            post_target: false,
            ..Entry::anchored()
        }
    }

    #[test]
    fn complete_supply_reserves_a_slot_without_reordering_sources() {
        let entries = [collected("items"), Entry::anchored()];
        let plan = route(&entries, &[Input::Positional, Input::Named("items")]).unwrap();
        assert_eq!(plan.routes[0].parameter, 1);
        assert_eq!(plan.routes[0].sources, [0]);
        assert_eq!(plan.routes[1].parameter, 0);
        assert_eq!(plan.routes[1].sources, [1]);
        assert!(!plan.routes[1].collected);
        assert_eq!(plan.assigned, [true, true]);

        let plan = route(&entries, &[Input::Positional, Input::Positional]).unwrap();
        assert_eq!(plan.routes[0].parameter, 0);
        assert!(plan.routes[0].collected);
        assert_eq!(plan.routes[0].sources, [0]);
        assert_eq!(plan.routes[1].parameter, 1);
        assert_eq!(plan.routes[1].sources, [1]);
    }

    #[test]
    fn routing_failure_does_not_mutate_the_interface() {
        let entries = [collected("items")];
        for (inputs, failure) in [
            (
                vec![Input::Named("items"), Input::Positional],
                FailureKind::AppendToComplete.call(),
            ),
            (
                vec![Input::Positional, Input::Named("items")],
                FailureKind::AppendToComplete.call(),
            ),
            (
                vec![Input::Named("items"), Input::Named("items")],
                FailureKind::Duplicate.at(1),
            ),
            (
                vec![Input::Named("unknown")],
                FailureKind::UnknownName.at(0),
            ),
        ] {
            assert_eq!(route(&entries, &inputs).unwrap_err(), failure);
            let recovered = route(&entries, &[Input::Named("items")]).unwrap();
            assert!(!recovered.routes[0].collected);
            assert_eq!(recovered.routes[0].sources, [0]);
        }
        let empty = route(&entries, &[]).unwrap();
        assert!(empty.routes[0].collected);
        assert!(empty.routes[0].sources.is_empty());
    }

    #[test]
    fn post_lambda_selection_preserves_explicit_and_anchored_entries() {
        let mut entries = [Entry::<&str>::anchored(), Entry::anchored()];
        assert_eq!(
            post_lambda_target(&entries, &[Input::PostLambda]),
            Ok(Some(1))
        );
        entries[0].lambda = true;
        let plan = route(&entries, &[Input::Positional, Input::PostLambda]).unwrap();
        assert_eq!(plan.routes[0].parameter, 1);
        assert_eq!(plan.routes[1].parameter, 0);
        assert!(plan.routes[1].lambda);
        entries[1].lambda = true;
        assert_eq!(
            post_lambda_target(&entries, &[Input::PostLambda]),
            Err(FailureKind::AmbiguousPostLambda.call())
        );
        for entry in &mut entries {
            entry.lambda = false;
            entry.post_target = false;
        }
        assert_eq!(
            post_lambda_target(&entries, &[Input::PostLambda]),
            Err(FailureKind::MissingPostTarget.call())
        );
        assert_eq!(
            post_lambda_target(&entries, &[Input::PostLambda, Input::PostLambda]),
            Err(FailureKind::RepeatedPostLambda.call())
        );
    }

    #[test]
    fn omitted_default_is_distinct_from_missing_implicit_and_empty_collection() {
        let mut default = Entry {
            name: Some("fallback"),
            optional: true,
            post_target: false,
            ..Entry::anchored()
        };
        let plan = route(&[default.clone(), collected("items")], &[]).unwrap();
        assert_eq!(plan.assigned, [false, true]);
        assert_eq!(plan.routes.len(), 1);
        assert_eq!(plan.routes[0].parameter, 1);
        default.implicit = true;
        assert_eq!(
            route(&[default], &[]).unwrap_err(),
            FailureKind::MissingImplicit.call()
        );
    }

    #[test]
    fn dumps_keep_names_escaped_and_routes_in_source_order() {
        let entry = collected("items\"\n");
        assert_eq!(
            entry.to_string(),
            "(input-entry (name \"items\\\"\\n\") (optional false) (implicit false) (variadic true) (lambda false) (post-target false))"
        );
        let plan = route(
            &[Entry::<&str>::anchored(), collected("rest")],
            &[Input::Named("rest"), Input::Positional],
        )
        .unwrap();
        assert_eq!(
            plan.to_string(),
            "(input-routing (routes (input-route (parameter 1) (sources 0) (collected false) (lambda false)) (input-route (parameter 0) (sources 1) (collected false) (lambda false))) (assigned 0 1))"
        );
    }
}
