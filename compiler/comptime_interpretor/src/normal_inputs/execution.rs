//! Source evaluation is independent of when the receiving domain arrives.
//! Only literal conversion and aggregate construction depend on that domain.
use super::*;

use super::delivery::{Delivery, Ready, Staged};

enum Container<D: Delivery> {
    List(D::List),
    Tuple(Vec<D::Item>),
}

enum Work<'a, D: Delivery> {
    Enter(&'a A, InputKey),
    Exit(&'a A),
    LeaveScope,
    Fields {
        items: &'a [A],
        index: usize,
        key: InputKey,
        container: Container<D>,
    },
}

macro_rules! operand {
    ($result:expr) => {
        match $result? {
            Flow::Value(value) => value,
            flow => return Ok(ControlFlow::Break(flow)),
        }
    };
}

impl Interpreter<'_> {
    fn execute_source_input<D: Delivery>(
        &mut self,
        input: &A,
        environment: &mut Environment,
        initial_depth: usize,
        mut delivery: D,
    ) -> Result<ControlFlow<Flow, D::Item>, Error> {
        let mut open_scopes = 0;
        let result = self.with_depth_checkpoint(|vm| {
            let caller_depth = vm.depth;
            let mut work = vec![Work::<D>::Enter(input, InputKey::root())];
            let mut completed = None;
            while let Some(next) = work.pop() {
                match next {
                    Work::Fields {
                        items,
                        index,
                        key,
                        mut container,
                    } => {
                        if index != 0 {
                            let value = completed.take().expect("completed container field");
                            match &mut container {
                                Container::List(list) => {
                                    let field = &items[index - 1];
                                    D::append(
                                        list,
                                        vm,
                                        value,
                                        matches!(field, A::ListSpread(_)),
                                        field.source(),
                                    )?;
                                }
                                Container::Tuple(fields) => fields.push(value),
                            }
                        }
                        if let Some(item) = items.get(index) {
                            let child = key.child(index);
                            work.push(Work::Fields {
                                items,
                                index: index + 1,
                                key,
                                container,
                            });
                            work.push(Work::Enter(item, child));
                        } else {
                            completed = Some(match container {
                                Container::List(list) => D::finish_list(list, vm)?,
                                Container::Tuple(fields) => D::tuple(key, fields),
                            });
                        }
                    }
                    Work::Exit(input) => {
                        vm.depth -= 1;
                        // Every selected contextual child has already returned through
                        // its own exit. Only this node's unconditional operands remain.
                        if input.has_diverging_operand() {
                            return Err(vm.failure(ErrorKind::Type, Some(input.source())));
                        }
                    }
                    Work::LeaveScope => {
                        environment.leave_scope();
                        open_scopes -= 1;
                    }
                    Work::Enter(input, key) => {
                        if initial_depth.saturating_add(vm.depth - caller_depth) >= 128 {
                            return Err(vm.failure(ErrorKind::DepthExhausted, Some(input.source())));
                        }
                        vm.tick(Some(input.source()))?;
                        vm.depth += 1;
                        work.push(Work::Exit(input));
                        match input {
                            A::ListSpread(value) | A::Value(value) => {
                                completed = Some(operand!(vm.eval(value, environment)).into());
                            }
                            A::Integer { .. }
                            | A::Symbol { .. }
                            | A::Unit { .. }
                            | A::Null { .. } => {
                                completed = Some(delivery.literal(key, input.source())?);
                            }
                            A::List { items, .. } | A::Tuple { items, .. } => {
                                let container = if matches!(input, A::List { .. }) {
                                    Container::List(delivery.list(&key, input.source())?)
                                } else {
                                    Container::Tuple(Vec::new())
                                };
                                work.push(Work::Fields {
                                    items,
                                    index: 0,
                                    key,
                                    container,
                                });
                            }
                            A::If {
                                condition,
                                then_input,
                                else_input,
                                ..
                            } => {
                                let value = operand!(vm.eval(condition, environment));
                                let Value::Bool(flag) =
                                    Self::concrete_value(value, condition.source)?
                                else {
                                    return Err(vm.failure(ErrorKind::Type, Some(condition.source)));
                                };
                                work.push(Work::Enter(
                                    if flag { then_input } else { else_input },
                                    key.child(usize::from(!flag)),
                                ));
                            }
                            A::Block {
                                statements, result, ..
                            } => {
                                environment.begin_scope();
                                open_scopes += 1;
                                work.push(Work::LeaveScope);
                                for statement in statements {
                                    operand!(vm.eval(statement, environment));
                                }
                                match result {
                                    Some(result) => work.push(Work::Enter(result, key.child(0))),
                                    None => completed = Some(Value::Unit.into()),
                                }
                            }
                            A::Match {
                                source,
                                value,
                                arms,
                            } => {
                                let value = operand!(vm.eval(value, environment));
                                environment.begin_scope();
                                open_scopes += 1;
                                work.push(Work::LeaveScope);
                                let mut selected = None;
                                for (index, arm) in arms.iter().enumerate() {
                                    vm.tick(Some(*source))?;
                                    match vm.match_arm_guard(
                                        &arm.pattern,
                                        arm.guard.as_ref(),
                                        &value,
                                        environment,
                                    )? {
                                        Flow::Value(Value::Bool(false)) => {
                                            environment.leave_scope();
                                            environment.begin_scope();
                                        }
                                        Flow::Value(Value::Bool(true)) => {
                                            selected = Some((index, &arm.body));
                                            break;
                                        }
                                        flow => return Ok(ControlFlow::Break(flow)),
                                    }
                                }
                                let (index, child) = selected.ok_or_else(|| {
                                    vm.failure(ErrorKind::NonExhaustiveMatch, Some(*source))
                                })?;
                                work.push(Work::Enter(child, key.child(index)));
                            }
                        }
                    }
                }
            }
            Ok(ControlFlow::Continue(
                completed.expect("completed source input"),
            ))
        });
        // Transfers and failures discard bindings introduced in the input;
        // mutations to caller-owned bindings remain visible.
        for _ in 0..open_scopes {
            environment.leave_scope();
        }
        result
    }

    pub(crate) fn interpret_staged_argument(
        &mut self,
        input: &A,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow<Flow, StagedNormalInput>, Error> {
        self.execute_source_input(input, environment, depth, Staged)
    }

    pub(crate) fn eval_normal_argument(
        &mut self,
        input: &A,
        literals: &mut BTreeMap<InputKey, PreparedNormalInput>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        Ok(
            match self.execute_source_input(input, environment, 0, Ready(literals))? {
                ControlFlow::Continue(value) => Flow::Value(value),
                ControlFlow::Break(flow) => flow,
            },
        )
    }
}
