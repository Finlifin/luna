//! Arithmetic keeps protocol probes, checked operands and rollback in the
//! source region. Context search never grants permission to execute operands.
use super::*;
use driver::{Request, Step};
use operators::peers;

pub(in crate::program) fn operator(expr: &Expr<'_>) -> Option<crate::arithmetic::Operator> {
    use crate::arithmetic::Operator as O;
    let H::Binary(op, _, _) = expr.kind else {
        return None;
    };
    Some(match op {
        BinOp::Add => O::Add,
        BinOp::Sub => O::Subtract,
        BinOp::Mul => O::Multiply,
        BinOp::Div => O::Divide,
        BinOp::Rem => O::Remainder,
        _ => return None,
    })
}

/// Definition checking supplies descriptions for operands whose source may
/// still contain deferred domains. Drive the same protocol and peer search as
/// an ordinary expression, including recovery only at an active probe.
pub(in crate::program) fn describe<'h>(
    checker: &mut Checker<'_, '_>,
    expr: &Expr<'h>,
    expected: Option<&Term>,
    mut operand: impl FnMut(
        &mut Checker<'_, '_>,
        &Expr<'h>,
        Option<&Term>,
        usize,
    ) -> Result<n::Expr, Error>,
) -> Result<n::Expr, Error> {
    let destination = Destination::ordinary(
        checker,
        OrdinaryTail {
            expected,
            partial: None,
            input: None,
        },
        expr.span,
    )?;
    let mut frame = Box::new(Frame::new(checker, expr, destination, true)?);
    let mut returned = None;
    loop {
        match frame.step(checker, returned.take())? {
            Step::Continue => {}
            Step::Child(Request::Expression(input, destination)) => {
                let slot = match frame.pending.expect("requested operand") {
                    Child::Left => 0,
                    Child::Right | Child::NativeRight => 1,
                    Child::Peer(slot) => slot,
                };
                match operand(checker, &input, destination.expected(), slot) {
                    Ok(value) => returned = Some(CheckedBlock::Ordinary(value)),
                    Err(failure) => frame.recover(checker, failure)?,
                }
            }
            Step::Completed(result) => return result.ordinary(expr.span),
            _ => unreachable!("arithmetic only requests expressions"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Start,
    InspectLeft,
    NativeRight,
    ContextRight,
    Select,
    Protocol,
    Peers,
    Finished,
}

#[derive(Clone, Copy, Debug)]
enum Child {
    Left,
    Right,
    NativeRight,
    Peer(usize),
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    inputs: [&'h Expr<'h>; 2],
    destination: Destination,
    protocol: Option<DeclarationId>,
    phase: Phase,
    pending: Option<Child>,
    attempt: Option<checking_attempts::Attempt>,
    native: bool,
    native_failure: Option<Error>,
    left_probed: bool,
    values: [Option<n::Expr>; 2],
    peers: Option<peers::Progress<'h, 'h>>,
    preserve_effect: Option<bool>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let op = operator(source)
            .ok_or_else(|| error(source.span, "expected an arithmetic operator"))?;
        let H::Binary(_, left, right) = source.kind else {
            unreachable!()
        };
        if destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "arithmetic requires an ordinary destination",
            ));
        }
        Ok(Self {
            source: source.clone(),
            inputs: [left, right],
            destination,
            protocol: checker.protocols.arithmetic[op as usize]
                .or(checker.protocols.comptime_arithmetic[op as usize]),
            phase: Phase::Start,
            pending: None,
            attempt: None,
            native: false,
            native_failure: None,
            left_probed: false,
            values: [None, None],
            peers: None,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        })
    }

    fn request(
        &mut self,
        checker: &mut Checker<'_, '_>,
        child: Child,
        slot: usize,
        expected: Option<&Term>,
        attempt: bool,
    ) -> Result<Step<'h>, Error> {
        let input = self.inputs[slot];
        let destination = Destination::ordinary(
            checker,
            OrdinaryTail {
                expected,
                partial: None,
                input: None,
            },
            input.span,
        )?;
        if attempt {
            self.attempt = Some(checking_attempts::Attempt::begin(
                checker,
                input.span,
                false,
                CheckingDisposition::Commit,
            )?);
        }
        self.pending = Some(child);
        Ok(Step::Child(Request::expression(input, destination)))
    }

    fn accept(&mut self, value: Option<n::Expr>) -> Result<(), Error> {
        match self
            .pending
            .take()
            .ok_or_else(|| error(self.source.span, "unsolicited arithmetic operand"))?
        {
            Child::Left => self.values[0] = value,
            Child::Right | Child::NativeRight => self.values[1] = value,
            Child::Peer(_) => self
                .peers
                .as_mut()
                .expect("active peer search")
                .accept(value)?,
        }
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        if let Some(value) = returned {
            let value = value.ordinary(self.source.span)?;
            let value = match self.attempt.take() {
                Some(attempt) => attempt.finish(checker, Ok(value))?,
                None => Some(value),
            };
            self.accept(value)?;
        }
        if self.pending.is_some() {
            return Err(error(
                self.source.span,
                "arithmetic operand is still pending",
            ));
        }
        match self.phase {
            Phase::Start => {
                if self.protocol.is_some() {
                    self.left_probed = true;
                    self.phase = Phase::InspectLeft;
                    return self.request(checker, Child::Left, 0, None, true);
                }
                self.begin_peers(checker)?;
            }
            Phase::InspectLeft => {
                let Some(value) = self.values[0].as_ref().filter(|value| !is_never(&value.ty))
                else {
                    self.begin_peers(checker)?;
                    return Ok(Step::Continue);
                };
                let shape = if matches!(
                    value.ty,
                    Term::Bound { .. }
                        | Term::AssociatedType { .. }
                        | Term::Rigid { .. }
                        | Term::NormalForm { .. }
                        | Term::Hole { .. }
                ) {
                    match checker
                        .context
                        .structural_view_with_heads_metered(
                            &value.ty,
                            crate::ptc::Budget::default(),
                            |head| Some(head) == checker.protocols.tuple,
                        )
                        .0
                    {
                        Ok(view) => view.map(|(ty, _)| ty).unwrap_or_else(|| value.ty.clone()),
                        Err(crate::ptc::UnknownReason::UnvalidatedModel) => value.ty.clone(),
                        Err(reason) => {
                            return Err(error(
                                self.inputs[0].span,
                                format!("arithmetic operand view remains unresolved: {reason:?}"),
                            ));
                        }
                    }
                } else {
                    value.ty.clone()
                };
                let H::Binary(op, _, _) = &self.source.kind else {
                    unreachable!()
                };
                self.native = checker.native_arithmetic_type(op, &shape)
                    || checker.native_arithmetic_type(op, &value.ty);
                self.phase = Phase::NativeRight;
                // A native left operand does not establish a homogeneous RHS.
                return self.request(checker, Child::Right, 1, None, true);
            }
            Phase::NativeRight => {
                self.phase = Phase::ContextRight;
                if self.values[1].is_none() && self.native {
                    let expected = checker
                        .copy_alias_type(&self.values[0].as_ref().unwrap().ty, self.source.span)?;
                    return self.request(checker, Child::NativeRight, 1, Some(&expected), true);
                }
            }
            Phase::ContextRight => {
                self.phase = Phase::Select;
                if self.values[1].is_none() {
                    let op = operator(&self.source).unwrap();
                    let left_ty = &self.values[0].as_ref().unwrap().ty;
                    if let Some(protocol) =
                        checker.selected_arithmetic_protocol(op, left_ty, self.source.span)?
                        && let Some(rhs) = checker.protocol_argument_context(
                            left_ty,
                            protocol,
                            self.source.span,
                        )?
                    {
                        return self.request(checker, Child::Right, 1, Some(&rhs), false);
                    }
                    if let Some(mut failure) = self.native_failure.take() {
                        failure
                            .message
                            .push_str("; arithmetic RHS has no unique protocol context");
                        return Err(failure);
                    }
                }
            }
            Phase::Select => {
                let left = self.values[0].as_ref().unwrap();
                let homogeneous = match self.values[1].as_ref() {
                    Some(right) if !is_never(&right.ty) && right.ty != left.ty => {
                        match checker.context.prove(
                            crate::ptc::Formula::equal(left.ty.clone(), right.ty.clone()),
                            crate::ptc::Budget::default(),
                        ) {
                            crate::ptc::Outcome::Entailed(_) => true,
                            crate::ptc::Outcome::NotEntailed(_)
                            | crate::ptc::Outcome::Unknown {
                                reason: crate::ptc::UnknownReason::UnvalidatedModel,
                                ..
                            } => false,
                            outcome => {
                                return Err(error(
                                    self.source.span,
                                    format!(
                                        "arithmetic operand equality remains unresolved: {outcome}"
                                    ),
                                ));
                            }
                        }
                    }
                    _ => true,
                };
                if !self.native || !homogeneous {
                    self.phase = Phase::Protocol;
                    if self.values[1].is_none() {
                        return self.request(checker, Child::Right, 1, None, false);
                    }
                } else {
                    self.begin_peers(checker)?;
                }
            }
            Phase::Protocol => {
                let op = operator(&self.source).unwrap();
                let left = self.values[0].take().unwrap();
                let right = self.values[1].take().unwrap();
                let protocol = checker
                    .selected_arithmetic_protocol(op, &left.ty, self.source.span)?
                    .ok_or_else(|| {
                        error(
                            self.source.span,
                            "arithmetic operator requires a bound protocol",
                        )
                    })?;
                let value = checker.finish_binary_protocol_call(
                    &self.source,
                    protocol,
                    op.method(),
                    left,
                    right,
                    self.destination.expected(),
                )?;
                return self.finish(checker, value);
            }
            Phase::Peers => {
                if let Some(request) = self.peers.as_mut().unwrap().next()? {
                    let destination = Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: request.expected,
                            partial: None,
                            input: None,
                        },
                        request.input.span,
                    )?;
                    if request.attempt {
                        self.attempt = Some(checking_attempts::Attempt::begin(
                            checker,
                            request.input.span,
                            false,
                            CheckingDisposition::Commit,
                        )?);
                    }
                    self.pending = Some(Child::Peer(request.slot));
                    return Ok(Step::Child(Request::expression(request.input, destination)));
                }
                let (left, right) = self.peers.take().unwrap().finish()?;
                let value = checker.finish_arithmetic_expression(
                    &self.source,
                    left,
                    right,
                    self.destination.expected(),
                )?;
                return self.finish(checker, value);
            }
            Phase::Finished => return Err(error(self.source.span, "arithmetic already completed")),
        }
        Ok(Step::Continue)
    }

    fn begin_peers(&mut self, checker: &mut Checker<'_, '_>) -> Result<(), Error> {
        let hint =
            checker.arithmetic_operand_hint(self.destination.expected(), self.source.span)?;
        let fallback = (is_numeric_literal(self.inputs[0]) && !is_float_literal(self.inputs[0]))
            .then(|| p(P::Integer));
        let ready = self.left_probed.then(|| std::mem::take(&mut self.values));
        self.peers = Some(peers::Progress::new(
            checker,
            self.inputs,
            hint.as_ref(),
            fallback.as_ref(),
            ready,
        )?);
        self.phase = Phase::Peers;
        Ok(())
    }

    fn finish(
        &mut self,
        checker: &mut Checker<'_, '_>,
        mut value: n::Expr,
    ) -> Result<Step<'h>, Error> {
        self.phase = Phase::Finished;
        if self.preserve_effect == Some(false) {
            value =
                checker.run_computation(value, self.destination.expected(), self.source.span)?;
        }
        Ok(Step::Completed(CheckedBlock::Ordinary(value)))
    }

    pub(super) fn recover(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(attempt) = self.attempt.take() else {
            return Err(failure);
        };
        match attempt.finish::<n::Expr>(checker, Err(failure)) {
            Ok(value) => self.accept(value),
            Err(failure)
                if matches!(self.pending, Some(Child::NativeRight))
                    && failure.incompatible_context =>
            {
                // Only this explicit native-context probe may recover an
                // incompatible domain. Semantic and budget errors still escape.
                self.native_failure = Some(failure);
                self.accept(None)
            }
            Err(failure) => Err(failure),
        }
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(arithmetic-frame (source {}) (phase {:?}) {} (pending",
            source(self.source.span),
            self.phase,
            self.destination
        )?;
        if let Some(child) = self.pending {
            match child {
                Child::Peer(slot) => write!(f, " (peer {slot})")?,
                _ => write!(f, " {child:?}")?,
            }
        }
        write!(
            f,
            ") (native {}) (left-probed {}) (protocol",
            self.native, self.left_probed
        )?;
        if let Some(protocol) = self.protocol {
            write!(f, " {protocol}")?;
        }
        f.write_str(") (effect-delivery")?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " (preserve {preserve})")?;
        }
        f.write_str(") (native-failure")?;
        if let Some(failure) = &self.native_failure {
            write!(
                f,
                " (source {}) (message {:?})",
                failure.source, failure.message
            )?;
        }
        f.write_str(") (operands")?;
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
        f.write_str(") (peers")?;
        if let Some(peers) = &self.peers {
            write!(f, " {peers}")?;
        }
        f.write_str(") (attempt")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " {attempt}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_dump_retains_checked_operand_and_failed_native_context() {
        let input = Expr {
            hir_id: hir::HirId::new(
                hir::OwnerId::new(hir::LocalDefId::new(1)),
                hir::ItemLocalId::new(2),
            ),
            span: Span::default(),
            kind: H::Unit,
        };
        let mut frame = Frame {
            source: input.clone(),
            inputs: [&input, &input],
            destination: Destination::statement(),
            protocol: Some(DeclarationId {
                package: 2,
                local: 3,
            }),
            phase: Phase::ContextRight,
            pending: Some(Child::NativeRight),
            attempt: None,
            native: true,
            native_failure: Some(error(input.span, "need \"enum\"\ncontext")),
            left_probed: true,
            values: [
                Some(n::Expr {
                    source: source(input.span),
                    ty: p(P::Bool),
                    kind: K::Literal(n::Literal::Bool(true)),
                }),
                None,
            ],
            peers: None,
            preserve_effect: Some(false),
        };
        let dump = frame.to_string();
        for text in [
            "(arithmetic-frame",
            "(phase ContextRight)",
            "(pending NativeRight)",
            "(native true)",
            "(preserve false)",
            "need \\\"enum\\\"\\ncontext",
            "(checked)",
        ] {
            assert!(dump.contains(text), "missing {text}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        frame.accept(None).unwrap();
        let retained = frame.to_string();
        assert!(frame.accept(None).is_err());
        assert_eq!(
            frame.to_string(),
            retained,
            "unsolicited result altered the frame"
        );
        assert!(frame.values[0].is_some());
        frame.pending = Some(Child::Peer(1));
        assert!(frame.to_string().contains("(pending (peer 1))"));
    }
}
