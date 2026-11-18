//! Synchronous drivers for focused executor comparisons in unit tests only.
//! Production input collection yields InputFrame to the invocation session;
//! it must not recurse through these borrowed-frame completion adapters.
use super::*;
use bytecode::{InputFrame, InputResult};
use comptime_compiler::Program;
use std::ops::ControlFlow;

impl InputFrame {
    pub(super) fn run(mut self, vm: &mut Interpreter<'_>) -> InputResult {
        let flow = vm.run_fragment(&mut self.code);
        self.finish.complete(vm, self.code, flow)
    }
}

impl Interpreter<'_> {
    pub(super) fn execute_code(
        &mut self,
        code: &Arc<Program>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut frame = self.start_frame(code, environment)?;
        let result = self.run_fragment(&mut frame);
        *environment = frame.restore_environment();
        result
    }

    pub(super) fn stage_normal_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<ControlFlow<Flow, normal_inputs::StagedNormalInput>, Error> {
        let Some(frame) = self.start_staged_input(input, environment, depth)? else {
            return self.interpret_staged_argument(input, environment, depth);
        };
        let result = frame.run(self);
        *environment = result.environment;
        Ok(match result.flow? {
            Flow::Value(_) => {
                let staged = result.staged.expect("completed staged fragment");
                std::ops::ControlFlow::Continue(staged)
            }
            flow => std::ops::ControlFlow::Break(flow),
        })
    }

    pub(super) fn evaluate_contextual_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if let Some(frame) = self.start_contextual_input(input, expected, environment)? {
            let result = frame.run(self);
            *environment = result.environment;
            return result.flow;
        }
        self.interpret_contextual_input(input, expected, environment)
    }

    pub(super) fn evaluate_input_preparation(
        &mut self,
        preparation: &nir::program::NormalFormPreparation,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let Some(frame) = self.start_input_preparation(preparation, environment)? else {
            return self.eval(&preparation.expression, environment);
        };
        let result = frame.run(self);
        *environment = result.environment;
        result.flow
    }
}
