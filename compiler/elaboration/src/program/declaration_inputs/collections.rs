//! Collector preparation keeps source positions and speculative checking exits.
//! The input driver supplies its domain and checks each requested source child.
use super::*;
use request::Request;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    Ordinary,
    Probe,
    RetryContextual,
    ContextualTail,
    ContextualLiteral,
    Complete,
    Failed,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ready => "ready",
            Self::Ordinary => "ordinary",
            Self::Probe => "probe",
            Self::RetryContextual => "retry-contextual",
            Self::ContextualTail => "contextual-tail",
            Self::ContextualLiteral => "contextual-literal",
            Self::Complete => "complete",
            Self::Failed => "failed",
        })
    }
}

pub(super) enum Poll<'b, 'h> {
    Child(Request<'b, 'h>),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    span: Span,
    checking: InputChecking,
    slot: Option<usize>,
    inputs: std::vec::IntoIter<PendingDeclarationCollection<'b, 'h>>,
    tail_start: usize,
    domain: Option<DeclarationInputDomain>,
    pending: Option<PendingDeclarationCollection<'b, 'h>>,
    state: State,
    attempt: Option<checking_attempts::Attempt>,
    collected: Vec<n::NormalFormArgument>,
    preparations: Vec<Vec<n::NormalFormPreparation>>,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        span: Span,
        checking: InputChecking,
        slot: Option<usize>,
        source_count: usize,
        inputs: Vec<PendingDeclarationCollection<'b, 'h>>,
        preparations: Vec<Vec<n::NormalFormPreparation>>,
    ) -> Self {
        // Only a contiguous source suffix executes inside the final collector.
        // Earlier producers retain their positions among fixed/empty packets.
        let mut tail_start = source_count;
        for input in inputs.iter().rev() {
            if input.source_index + 1 != tail_start {
                break;
            }
            tail_start = input.source_index;
        }
        Self {
            span,
            checking,
            slot,
            collected: Vec::with_capacity(inputs.len()),
            inputs: inputs.into_iter(),
            tail_start,
            domain: None,
            pending: None,
            state: State::Ready,
            attempt: None,
            preparations,
        }
    }

    pub(super) fn domain_slot(&self) -> Option<usize> {
        (self.domain.is_none() && !self.inputs.as_slice().is_empty())
            .then(|| self.slot.expect("collected inputs have a collector"))
    }

    pub(super) fn accept_domain(&mut self, domain: DeclarationInputDomain) -> Result<(), Error> {
        if self.domain_slot().is_none() || self.state != State::Ready {
            return Err(error(self.span, "collector has no pending domain"));
        }
        self.domain = Some(domain);
        Ok(())
    }

    pub(super) fn poll(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'b, 'h>, Error> {
        if self.state == State::RetryContextual {
            self.state = State::ContextualLiteral;
            return Ok(Poll::Child(Request::contextual(
                self.pending.as_ref().expect("contextual retry input").input,
            )));
        }
        if self.state != State::Ready || self.pending.is_some() {
            return Err(error(self.span, "collection check is pending or complete"));
        }
        // No input is consumed until the driver has delivered its domain.
        if self.domain_slot().is_some() {
            return Err(error(self.span, "collection check awaits its domain"));
        }
        while let Some(pending) = self.inputs.next() {
            let domain = self.domain.as_ref().expect("observed collection domain");
            let expected = if pending.spread {
                &domain.ty
            } else {
                checker.list_element_type(&domain.ty).ok_or_else(|| {
                    error(
                        pending.input.span,
                        "variadic input requires a bound meta.List<T>",
                    )
                })?
            };
            let contextual =
                domain.pending && !pending.spread && self.checking == InputChecking::Contextual;
            let state = if contextual {
                if pending.source_index >= self.tail_start {
                    State::ContextualTail
                } else if matches!(pending.input.kind, H::Unit) {
                    self.collected.push(n::NormalFormArgument::Unit {
                        source: source(pending.input.span),
                    });
                    continue;
                } else {
                    State::Probe
                }
            } else {
                State::Ordinary
            };
            let request = if state == State::ContextualTail {
                Request::contextual(pending.input)
            } else {
                Request::ordinary(
                    pending.input,
                    (!domain.pending)
                        .then(|| checker.copy_alias_type(expected, self.span))
                        .transpose()?,
                )
            };
            if state == State::Probe {
                // Begin after all fallible request preparation. Every returned
                // child, including a failure, passes through accept below.
                self.attempt = Some(checking_attempts::Attempt::begin(
                    checker,
                    pending.input.span,
                    false,
                    CheckingDisposition::Commit,
                )?);
            }
            self.pending = Some(pending);
            self.state = state;
            return Ok(Poll::Child(request));
        }
        self.state = State::Complete;
        Ok(Poll::Complete)
    }

    pub(super) fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| error(self.span, "collector has no pending child"))?;
        let state = std::mem::replace(&mut self.state, State::Failed);
        let value = if state == State::Probe {
            match self
                .attempt
                .take()
                .expect("collection checking attempt")
                .finish(checker, result)?
            {
                Some(value) => value,
                None => {
                    self.pending = Some(pending);
                    self.state = State::RetryContextual;
                    return Ok(());
                }
            }
        } else {
            result?
        };
        let value = match state {
            State::ContextualTail => value,
            State::ContextualLiteral => match value {
                value @ (n::NormalFormArgument::Integer { .. }
                | n::NormalFormArgument::Null { .. }) => value,
                _ => {
                    return Err(context_error(
                        pending.input.span,
                        "structured collection input requires an element type context",
                    ));
                }
            },
            State::Ordinary | State::Probe => {
                let n::NormalFormArgument::Value(value) = value else {
                    return Err(error(
                        pending.input.span,
                        "collector requires an ordinary expression",
                    ));
                };
                let value = if pending.source_index >= self.tail_start {
                    value
                } else {
                    checker.prepare_declaration_collection_input(
                        value,
                        pending.before,
                        &mut self.preparations[pending.source_index],
                        bindings,
                        self.span,
                    )?
                };
                if pending.spread {
                    n::NormalFormArgument::ListSpread(value)
                } else {
                    n::NormalFormArgument::Value(value)
                }
            }
            _ => return Err(error(self.span, "collector is not awaiting a child result")),
        };
        self.collected.push(value);
        self.state = State::Ready;
        Ok(())
    }

    pub(super) fn finish(
        self,
    ) -> Result<
        (
            Option<usize>,
            Vec<n::NormalFormArgument>,
            Vec<n::NormalFormPreparation>,
        ),
        Error,
    > {
        if self.state != State::Complete || self.pending.is_some() || self.attempt.is_some() {
            return Err(error(self.span, "collection checking is incomplete"));
        }
        Ok((
            self.slot,
            self.collected,
            self.preparations.into_iter().flatten().collect(),
        ))
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-collection-frame (state {}) (tail-start {}) (slot",
            self.state, self.tail_start
        )?;
        if let Some(slot) = self.slot {
            write!(f, " {slot}")?;
        }
        f.write_str(") (domain")?;
        if let Some(domain) = &self.domain {
            write!(f, " {domain}")?;
        }
        f.write_str(") (pending")?;
        if let Some(pending) = &self.pending {
            write!(f, " {pending}")?;
        }
        f.write_str(") (remaining")?;
        for input in self.inputs.as_slice() {
            write!(f, " {input}")?;
        }
        f.write_str(") (attempt")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " {attempt}")?;
        }
        f.write_str(") (collected")?;
        for value in &self.collected {
            write!(f, " {value}")?;
        }
        f.write_str(") (preparations")?;
        for (source_index, preparations) in self.preparations.iter().enumerate() {
            for preparation in preparations {
                write!(
                    f,
                    " (source-index {source_index} (before {}) {})",
                    preparation.before, preparation.expression
                )?;
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_frame_retains_packet_positions_and_rejects_incomplete_publication() {
        let span = Span::default();
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let input = Expr {
            hir_id: hir::HirId::new(owner, hir::ItemLocalId::new(2)),
            span,
            kind: H::Unit,
        };
        let inputs = [(1, false), (3, true), (5, false)]
            .into_iter()
            .map(|(source_index, spread)| PendingDeclarationCollection {
                source_index,
                before: source_index / 2,
                input: &input,
                spread,
            })
            .collect();
        let mut preparations = vec![vec![]; 6];
        preparations[4].push(n::NormalFormPreparation {
            before: 2,
            expression: n::Expr {
                source: source(span),
                ty: p(P::Str),
                kind: K::Literal(n::Literal::String("packet\"\n".into())),
            },
        });
        let frame = Frame::new(
            span,
            InputChecking::Contextual,
            Some(2),
            6,
            inputs,
            preparations,
        );
        assert_eq!(frame.tail_start, 5);
        assert_eq!(frame.domain_slot(), Some(2));
        let dump = frame.to_string();
        for expected in [
            "(state ready)",
            "(source-index 3) (before 1) (spread true)",
            "(preparations (source-index 4 (before 2)",
            "packet\\\"\\n",
        ] {
            assert!(dump.contains(expected), "missing {expected}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        assert!(frame.finish().is_err());
    }
}
