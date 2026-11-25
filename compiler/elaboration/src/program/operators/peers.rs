//! One operand-context search shared by arithmetic and comparison region frames.
//! Checked operands remain code; none of these transitions executes a producer.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Phase {
    Left,
    Right,
    LeftPeer,
    RightPeer,
    DefaultLeft,
    DefaultRight,
    Complete,
}

pub(in crate::program) struct Progress<'b, 'h> {
    inputs: [&'b Expr<'h>; 2],
    expected: Option<Term>,
    fallback: Option<Term>,
    defaults: bool,
    phase: Phase,
    waiting: Option<usize>,
    values: [Option<n::Expr>; 2],
}

pub(in crate::program) struct Request<'a, 'b, 'h> {
    pub slot: usize,
    pub input: &'b Expr<'h>,
    pub expected: Option<&'a Term>,
    pub attempt: bool,
}

impl<'b, 'h> Progress<'b, 'h> {
    pub(in crate::program) fn new(
        checker: &mut Checker<'_, '_>,
        inputs: [&'b Expr<'h>; 2],
        expected: Option<&Term>,
        fallback: Option<&Term>,
        ready: Option<[Option<n::Expr>; 2]>,
    ) -> Result<Self, Error> {
        Ok(Self {
            inputs,
            expected: expected
                .map(|ty| checker.copy_alias_type(ty, inputs[0].span))
                .transpose()?,
            fallback: fallback
                .map(|ty| checker.copy_alias_type(ty, inputs[0].span))
                .transpose()?,
            defaults: checker.numeric_defaults,
            phase: if ready.is_some() {
                Phase::Right
            } else {
                Phase::Left
            },
            waiting: None,
            values: ready.unwrap_or([None, None]),
        })
    }

    pub(in crate::program) fn next(&mut self) -> Result<Option<Request<'_, 'b, 'h>>, Error> {
        if self.waiting.is_some() {
            return Err(error(self.inputs[0].span, "operand check is still pending"));
        }
        loop {
            let phase = self.phase;
            let (slot, attempt) = match phase {
                Phase::Left => {
                    self.phase = Phase::Right;
                    (0, true)
                }
                Phase::Right => {
                    self.phase = Phase::LeftPeer;
                    (1, true)
                }
                Phase::LeftPeer => {
                    self.phase = Phase::RightPeer;
                    (0, true)
                }
                Phase::RightPeer => {
                    self.phase = Phase::DefaultLeft;
                    (1, true)
                }
                Phase::DefaultLeft => {
                    if self.values.iter().any(Option::is_none) && !self.defaults {
                        return Err(context_error(
                            self.inputs[0].span,
                            "numeric operands await a shared type context",
                        ));
                    }
                    self.phase = Phase::DefaultRight;
                    (0, false)
                }
                Phase::DefaultRight => {
                    self.phase = Phase::Complete;
                    (1, false)
                }
                Phase::Complete => return Ok(None),
            };
            if self.values[slot].is_some() {
                continue;
            }
            let peer = self.values[1 - slot]
                .as_ref()
                .filter(|value| !is_never(&value.ty))
                .map(|value| &value.ty);
            let expected = match phase {
                Phase::Left => self.expected.as_ref(),
                Phase::Right => self.expected.as_ref().or(peer),
                Phase::LeftPeer | Phase::RightPeer => {
                    if peer.is_none() {
                        continue;
                    }
                    peer
                }
                Phase::DefaultLeft | Phase::DefaultRight => {
                    self.expected.as_ref().or(peer).or(self.fallback.as_ref())
                }
                Phase::Complete => unreachable!(),
            };
            self.waiting = Some(slot);
            return Ok(Some(Request {
                slot,
                input: self.inputs[slot],
                expected,
                attempt,
            }));
        }
    }

    pub(in crate::program) fn accept(&mut self, value: Option<n::Expr>) -> Result<(), Error> {
        let slot = self
            .waiting
            .take()
            .ok_or_else(|| error(self.inputs[0].span, "unsolicited operand result"))?;
        self.values[slot] = value;
        Ok(())
    }

    pub(in crate::program) fn peer_type(&self, slot: usize) -> Option<&Term> {
        self.values[1 - slot]
            .as_ref()
            .filter(|value| !is_never(&value.ty))
            .map(|value| &value.ty)
    }

    pub(in crate::program) fn finish(self) -> Result<(n::Expr, n::Expr), Error> {
        match (self.phase, self.waiting, self.values) {
            (Phase::Complete, None, [Some(left), Some(right)]) => Ok((left, right)),
            _ => Err(error(self.inputs[0].span, "operand checking is incomplete")),
        }
    }
}

impl std::fmt::Display for Progress<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(peer-operands (phase {:?}) (defaults {}) (waiting",
            self.phase, self.defaults
        )?;
        if let Some(slot) = self.waiting {
            write!(f, " {slot}")?;
        }
        f.write_str(") (expected")?;
        if let Some(ty) = &self.expected {
            write!(f, " {ty}")?;
        }
        f.write_str(") (fallback")?;
        if let Some(ty) = &self.fallback {
            write!(f, " {ty}")?;
        }
        f.write_str(") (inputs")?;
        for (input, value) in self.inputs.iter().zip(&self.values) {
            write!(
                f,
                " (operand {} (source {}) (checked",
                input.hir_id,
                source(input.span)
            )?;
            if let Some(value) = value {
                write!(f, " {value}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_progress_dump_distinguishes_pending_source_and_checked_code() {
        let input = Expr {
            hir_id: hir::HirId::new(
                hir::OwnerId::new(hir::LocalDefId::new(1)),
                hir::ItemLocalId::new(2),
            ),
            span: Span::default(),
            kind: H::Unit,
        };
        let progress = Progress {
            inputs: [&input, &input],
            expected: None,
            fallback: Some(p(P::Str)),
            defaults: false,
            phase: Phase::LeftPeer,
            waiting: Some(1),
            values: [
                Some(n::Expr {
                    source: source(input.span),
                    ty: p(P::Str),
                    kind: K::Literal(n::Literal::String("left\"\n".into())),
                }),
                None,
            ],
        };
        let dump = progress.to_string();
        assert!(dump.starts_with("(peer-operands "), "{dump}");
        for fragment in [
            "(phase LeftPeer)",
            "(waiting 1)",
            "(defaults false)",
            "(expected)",
            "(fallback (primitive str))",
            "(checked)",
            "left\\\"\\n",
        ] {
            assert!(dump.contains(fragment), "missing {fragment}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
    }
}
