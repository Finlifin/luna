//! Dependency checks retain their consumer and child position. Checking a later
//! source input first never changes the execution schedule owned by Progress.
use super::*;

pub(super) enum Poll {
    Continue,
    Input {
        slot: usize,
        domain: DeclarationInputDomain,
    },
    Complete(DeclarationInputDomain),
}

/// Edges refer to callee slots, never to names in the caller's replacement
/// terms. External declaration inputs remain in their enclosing telescope.
pub(in crate::program) struct Telescope {
    pub(super) parameters: Parameters,
    pub(super) positions: BTreeMap<DeclarationId, usize>,
    edges: Vec<Vec<usize>>,
    pub(super) default_captures: DeclarationBindings<n::LocalId>,
    pub(super) waiting_captures: BTreeMap<usize, Vec<DeclarationId>>,
}

impl Telescope {
    pub(in crate::program) fn new(
        parameters: Parameters,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        environment::charge(fuel, parameters.len(), span)?;
        let positions: BTreeMap<_, _> = parameters
            .iter()
            .enumerate()
            .map(|(index, (id, _, _))| (*id, index))
            .collect();
        if parameters.len() != positions.len() {
            return Err(error(span, "duplicate declaration input identity"));
        }
        let mut edges = Vec::with_capacity(parameters.len());
        for (_, _, domain) in parameters.iter() {
            let refs = input_observation::term_inputs(domain, fuel, span)?.declarations;
            edges.push(
                refs.keys()
                    .filter_map(|id| positions.get(id).copied())
                    .collect(),
            );
        }
        Ok(Self {
            parameters,
            positions,
            edges,
            default_captures: DeclarationBindings::default(),
            waiting_captures: BTreeMap::new(),
        })
    }

    /// Read only declaration templates. A rigid in a replacement belongs to
    /// the caller even when its spelling or declaration ID matches a capture.
    pub(in crate::program) fn track_captures(
        &mut self,
        templates: &[Term],
        dynamic: DeclarationBindings<n::LocalId>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        if templates.len() != self.parameters.len() {
            return Err(error(span, "capture domain telescope arity mismatch"));
        }
        if !dynamic.is_empty() {
            for (slot, template) in templates.iter().enumerate() {
                let refs = input_observation::term_inputs(template, fuel, span)?.declarations;
                let waiting: Vec<_> = refs
                    .into_keys()
                    .filter(|id| dynamic.contains_key(id))
                    .collect();
                if !waiting.is_empty() {
                    self.waiting_captures.insert(slot, waiting);
                }
            }
        }
        self.default_captures = dynamic;
        Ok(())
    }

    pub(super) fn independent(&self, slot: usize) -> bool {
        self.edges[slot].is_empty() && !self.waiting_captures.contains_key(&slot)
    }

    pub(super) fn direct(&self, slot: usize) -> &[usize] {
        &self.edges[slot]
    }
}

/// Visit the telescope backwards to collect the required slots, then check
/// them in declaration order. An omitted default needs its lexical prefix;
/// keep that interval as a cursor instead of inserting every slot into a set.
enum Phase {
    Select,
    Edges { slot: usize, next: usize },
    Check,
    Complete,
}

pub(super) struct Frame {
    slot: usize,
    span: Span,
    phase: Phase,
    frontier: BTreeSet<usize>,
    default_prefix: std::ops::Range<usize>,
    checks: Vec<usize>,
}

enum Step {
    Continue,
    Check(usize),
    Consumer,
}

impl Frame {
    pub(super) fn new(slot: usize, prepared_prefix: usize, span: Span) -> Self {
        Self {
            slot,
            span,
            phase: Phase::Edges { slot, next: 0 },
            frontier: BTreeSet::new(),
            default_prefix: prepared_prefix..prepared_prefix,
            checks: Vec::new(),
        }
    }

    pub(super) fn slot(&self) -> usize {
        self.slot
    }

    fn predecessor(&mut self) -> Option<usize> {
        if !self.default_prefix.is_empty()
            && self
                .frontier
                .last()
                .is_none_or(|&slot| slot < self.default_prefix.end)
        {
            self.default_prefix.end -= 1;
            self.frontier.remove(&self.default_prefix.end);
            Some(self.default_prefix.end)
        } else {
            self.frontier.pop_last()
        }
    }

    fn advance(
        &mut self,
        edges: &[Vec<usize>],
        prepared: impl Fn(usize) -> bool,
        uses_default: impl Fn(usize) -> bool,
        fuel: &mut u64,
    ) -> Result<Step, Error> {
        environment::charge(fuel, 1, self.span)?;
        match self.phase {
            Phase::Edges { slot, ref mut next } => {
                if let Some(&input) = edges[slot].get(*next) {
                    *next += 1;
                    if input < slot && input >= self.default_prefix.end {
                        self.frontier.insert(input);
                    }
                } else {
                    self.phase = Phase::Select;
                }
            }
            Phase::Select => {
                if let Some(slot) = self.predecessor() {
                    // Its producer may still lack an identity. Rechecking its
                    // dependencies cannot supply that value; the original
                    // invocation owns it.
                    if !prepared(slot) {
                        self.checks.push(slot);
                        if uses_default(slot) {
                            self.default_prefix.end = slot;
                        }
                        self.phase = Phase::Edges { slot, next: 0 };
                    }
                } else {
                    self.phase = Phase::Check;
                }
            }
            Phase::Check => {
                if let Some(slot) = self.checks.pop() {
                    return Ok(Step::Check(slot));
                }
                self.phase = Phase::Complete;
                return Ok(Step::Consumer);
            }
            Phase::Complete => {
                return Err(error(self.span, "declaration domain check is complete"));
            }
        }
        Ok(Step::Continue)
    }

    pub(super) fn poll(
        &mut self,
        checker: &mut Checker<'_, '_>,
        progress: &progress::Progress,
        uses_default: impl Fn(usize) -> bool,
    ) -> Result<Poll, Error> {
        match self.advance(
            &progress.telescope.edges,
            |slot| progress.is_prepared(slot),
            uses_default,
            &mut checker.required_steps,
        )? {
            Step::Continue => Ok(Poll::Continue),
            Step::Check(slot) => Ok(Poll::Input {
                slot,
                domain: progress.domain(checker, slot, self.span)?,
            }),
            Step::Consumer => Ok(Poll::Complete(
                progress.domain(checker, self.slot, self.span)?,
            )),
        }
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-domain-frame (consumer {}) (source {}) (phase ",
            self.slot,
            source(self.span),
        )?;
        match self.phase {
            Phase::Select => f.write_str("select")?,
            Phase::Edges { slot, next } => write!(f, "(edges {slot} {next})")?,
            Phase::Check => f.write_str("check")?,
            Phase::Complete => f.write_str("complete")?,
        }
        write!(
            f,
            ") (default-prefix {} {}) (frontier",
            self.default_prefix.start, self.default_prefix.end
        )?;
        for slot in &self.frontier {
            write!(f, " {slot}")?;
        }
        f.write_str(") (checks")?;
        for slot in self.checks.iter().rev() {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 7, local }
    }

    fn dependency(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        }
    }

    fn check_order(
        frame: &mut Frame,
        edges: &[Vec<usize>],
        prepared: impl Fn(usize) -> bool,
        uses_default: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let mut order = Vec::new();
        let mut fuel = 200;
        loop {
            match frame
                .advance(edges, &prepared, &uses_default, &mut fuel)
                .unwrap()
            {
                Step::Continue => {}
                Step::Check(slot) => order.push(slot),
                Step::Consumer => return order,
            }
        }
    }

    #[test]
    fn overlapping_defaults_and_explicit_edges_preserve_declaration_order() {
        // 7 depends on 6 and 3; both also need 2. Slot 4 is unrelated
        // unless a default at 6 captures its declaration prefix.
        let edges = vec![
            vec![],
            vec![],
            vec![],
            vec![2],
            vec![],
            vec![],
            vec![2],
            vec![3, 6],
        ];
        let order = |defaults: &[usize]| {
            check_order(
                &mut Frame::new(7, 0, Span::default()),
                &edges,
                |_| false,
                |slot| defaults.contains(&slot),
            )
        };
        assert_eq!(order(&[]), [2, 3, 6]);
        assert_eq!(order(&[3]), [0, 1, 2, 3, 6]);
        assert_eq!(order(&[3, 6]), [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn prepared_inputs_cut_off_domain_and_default_dependencies() {
        let edges = vec![vec![], vec![], vec![], vec![0], vec![3, 1]];
        let order = check_order(
            &mut Frame::new(4, 0, Span::default()),
            &edges,
            |slot| slot == 3,
            |slot| {
                assert_ne!(
                    slot, 3,
                    "the prepared producer already owns its chosen supply"
                );
                false
            },
        );
        assert_eq!(order, [1]);
    }

    #[test]
    fn large_telescope_resumes_at_the_saved_edge_and_default_prefix() {
        let mut edges = vec![vec![]; 1026];
        edges[1025] = vec![600, 1024];
        let mut frame = Frame::new(1025, 0, Span::default());
        let advance = |frame: &mut Frame, fuel: &mut u64| {
            frame.advance(&edges, |_| false, |slot| slot == 1024, fuel)
        };
        assert!(matches!(
            advance(&mut frame, &mut 1).unwrap(),
            Step::Continue
        ));
        let saved = frame.to_string();
        assert!(saved.contains("(phase (edges 1025 1))"), "{saved}");
        assert!(advance(&mut frame, &mut 0).is_err());
        assert_eq!(frame.to_string(), saved);
        let mut fuel = 10_000;
        let mut next = 0;
        loop {
            match advance(&mut frame, &mut fuel).unwrap() {
                Step::Continue => {
                    assert!(frame.frontier.len() <= 2);
                }
                Step::Check(slot) => {
                    assert_eq!(slot, next);
                    next += 1;
                }
                Step::Consumer => break,
            }
        }
        assert_eq!(next, 1025);
    }

    #[test]
    fn an_omitted_default_only_visits_the_missing_declaration_suffix() {
        let mut edges = vec![vec![]; 1026];
        edges[1025] = vec![3, 1024];
        let order = check_order(
            &mut Frame::new(1025, 1020, Span::default()),
            &edges,
            |slot| {
                assert!(slot >= 1020, "checked prefix was traversed at slot {slot}");
                slot == 1022
            },
            |slot| slot == 1024,
        );
        assert_eq!(order, [1020, 1021, 1023, 1024]);
    }

    #[test]
    fn a_caller_replacement_does_not_introduce_a_pending_capture() {
        let templates = [dependency(7), dependency(8), dependency(9)];
        // The second domain acquired id(7) from a caller replacement. Only
        // the original reference in the first domain reads that capture.
        let parameters = (0..3)
            .map(|slot| (id(slot), Sort::ComptimeValue, dependency(7)))
            .collect();
        let mut fuel = 100;
        let span = Span::default();
        let mut telescope = Telescope::new(parameters, &mut fuel, span).unwrap();
        telescope
            .track_captures(
                &templates,
                DeclarationBindings::from([(id(7), n::LocalId { owner: 3, index: 0 })]),
                &mut fuel,
                span,
            )
            .unwrap();
        assert_eq!(
            telescope.waiting_captures,
            BTreeMap::from([(0, vec![id(7)])])
        );
    }

    #[test]
    fn domain_dependency_observation_rejects_conflicting_input_sorts() {
        let domain = Term::TupleValue(vec![
            dependency(0),
            Term::Rigid {
                declaration: id(0),
                sort: Sort::ComptimeValue,
            },
        ]);
        let parameters = [(id(0), Sort::Type, p(P::Type)), (id(1), Sort::Type, domain)];
        assert!(Telescope::new(parameters.into(), &mut 100, Span::default()).is_err());
    }
}
