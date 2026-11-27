//! Machine results at checked Flurry call boundaries. Function values always
//! point at the control-bearing entry; certified direct calls use the scalar body.
use super::*;

pub(super) const FAILURE: &str = "inttoptr (i32 1 to ptr)";

pub(super) const ABORT: &str = "inttoptr (i32 2 to ptr)";

pub(super) const LIKELY: &str = "!prof !{!\"branch_weights\", i32 2000, i32 1}";

pub(super) fn packet(payload: &Ty) -> Ty {
    Ty::Record(std::sync::Arc::new(vec![payload.clone(), Ty::Pointer]))
}

pub(super) fn scalar(instance: &iir::FunctionInstance) -> bool {
    let Term::Function { result, .. } = &instance.function.signature else {
        return false;
    };
    effects::row_result(result).0.is_empty()
        && instance
            .behavior
            .as_ref()
            .is_some_and(|b| b.no_trap_or_unwind)
}

impl Emitter<'_> {
    pub(super) fn control_token(&mut self) -> String {
        let token = self.fresh();
        self.prologue
            .push_str(&format!("  %{token} = alloca i8, align 2\n"));
        format!("%{token}")
    }

    pub(super) fn machine_return(&mut self, value: &Value, control: &str) {
        if control != "null" {
            self.emitted_effects.has_trap_or_unwind = true;
        }
        if self.failure_result.is_some() {
            if control != "null"
                && let Some(guard_slot) = self
                    .capturing_resume
                    .as_ref()
                    .map(|cap| cap.unconsumed_guard_slot.clone())
            {
                let pending_k =
                    self.instruction(Ty::Pointer, format!("load ptr, ptr {guard_slot}, align 8"));
                self.line(format!("store ptr null, ptr {guard_slot}, align 8"));
                self.line(format!(
                    "call void @flurry_cont_abort(ptr {}, ptr %task)",
                    pending_k.text
                ));
            }
            let ty = packet(&value.ty);
            let mut fields = "zeroinitializer".to_string();
            if value.ty != Ty::Unit && control == "null" {
                fields = self
                    .instruction(
                        ty.clone(),
                        format!(
                            "insertvalue {} zeroinitializer, {} {}, 0",
                            ty.llvm(),
                            value.ty.llvm(),
                            value.text
                        ),
                    )
                    .text;
            }
            if control != "null" {
                fields = self
                    .instruction(
                        ty.clone(),
                        format!("insertvalue {} {fields}, ptr {control}, 1", ty.llvm()),
                    )
                    .text;
            }
            self.line(format!("ret {} {fields}", ty.llvm()));
        } else if control != "null" {
            self.emit_trap();
            self.line("unreachable");
        } else if value.ty == Ty::Unit {
            self.line("ret void");
        } else {
            self.line(format!("ret {} {}", value.ty.llvm(), value.text));
        }
        self.terminated = true;
    }

    pub(super) fn control_return(&mut self, control: &str) {
        let ty = self.failure_result.clone().unwrap_or(Ty::Unit);
        self.machine_return(
            &Value {
                ty,
                text: String::new(),
            },
            control,
        );
    }

    /// Only a statically resolved symbol selects the scalar entry. A pointer
    /// remains callable even when two targets have different certifications.
    pub(super) fn machine_call(
        &mut self,
        target: &str,
        args: &[String],
        payload: Ty,
        suffix: &str,
    ) -> (Value, Option<Value>) {
        let fast = self.checked
            && Self::direct_instance_index(target)
                .and_then(|index| self.graph.functions.get(index))
                .is_some_and(scalar);
        let controlled = self.checked && !fast;
        let ty = if controlled {
            packet(&payload)
        } else {
            payload.clone()
        };
        let symbol = if fast {
            format!("{target}_scalar")
        } else {
            target.to_string()
        };
        let mut args = args.join(", ");
        if self.checked {
            if !args.is_empty() {
                args.push_str(", ");
            }
            args.push_str("ptr %task");
        }
        if !target.starts_with('@')
            || target.starts_with("@flurry_linux_")
            || target.starts_with("@flurry_task_")
            || target == "@dprintf"
        {
            self.emitted_effects.has_external_or_indirect_call = true;
        }
        let call = format!("call {} {symbol}({args}){suffix}", ty.llvm());
        if ty == Ty::Unit {
            self.line(call);
            return (Value::unit(), None);
        }
        let result = self.instruction(ty.clone(), call);
        if !controlled {
            return (result, None);
        }
        let extract = |field| format!("extractvalue {} {}, {field}", ty.llvm(), result.text);
        let control = self.instruction(Ty::Pointer, extract(1));
        let value = if payload == Ty::Unit {
            Value::unit()
        } else {
            self.instruction(payload, extract(0))
        };
        (value, Some(control))
    }

    pub(super) fn accept_control(
        &mut self,
        ctrl: &Value,
        source: nir::program::SourceRange,
        effects: bool,
    ) -> Result<(), String> {
        let ok = self.instruction(Ty::Bool, format!("icmp eq ptr {}, null", ctrl.text));
        let normal = self.fresh();
        let escape = self.fresh();
        self.line(format!(
            "br i1 {}, label %{normal}, label %{escape}, {}",
            ok.text, LIKELY
        ));
        self.label(&escape);
        if effects {
            let is_failure =
                self.instruction(Ty::Bool, format!("icmp eq ptr {}, {FAILURE}", ctrl.text));
            let is_abort =
                self.instruction(Ty::Bool, format!("icmp eq ptr {}, {ABORT}", ctrl.text));
            let failed = self.instruction(
                Ty::Bool,
                format!("or i1 {}, {}", is_failure.text, is_abort.text),
            );
            let failure = self.fresh();
            let transfer = self.fresh();
            self.line(format!(
                "br i1 {}, label %{failure}, label %{transfer}",
                failed.text
            ));
            self.label(&transfer);
            self.receive_effect_control(ctrl, source)?;
            self.label(&failure);
        }
        self.cleanup_boundary(source, iir::cleanup::Boundary::PredicateFailure)?;
        self.unwind_resources()?;
        self.control_return(&ctrl.text);
        self.label(&normal);
        Ok(())
    }
}

/// A foreign ABI caller owns neither a Flurry control continuation nor task-local state.
/// Admit only representations whose foreign ABI classification the host already knows.
pub(super) fn export_c_entries(
    graph: &iir::Package,
    attributes: &nir::program::DefinitionAttributes,
    module: &mut String,
    checked: bool,
    target_triple: Option<&str>,
    fuel: &mut usize,
) -> Result<(), String> {
    if attributes.is_empty() {
        return Ok(());
    }
    // Only published entries need a symbol check. Backend symbols here are bare
    // LLVM names; foreign export names (which may be Unicode) are emitted quoted.
    let mut names = None;
    for (index, instance) in graph.functions.iter().enumerate() {
        let function = &instance.function;
        *fuel = fuel
            .checked_sub(1)
            .ok_or("foreign ABI admission budget exhausted")?;
        let Some(convention) = attributes.calling_convention(function.declaration, fuel)? else {
            continue;
        };
        if !checked {
            return Err("foreign ABI boundaries require a cleanup-checked host graph".into());
        }
        let Term::Function { parameters, result } = &function.signature else {
            return Err("foreign ABI requires a concrete function".into());
        };
        let (row, result) = effects::row_result(result);
        if !row.is_empty()
            || !function.env_parameters.is_empty()
            || !instance.key.arguments.is_empty()
        {
            return Err("foreign ABI requires a closed function with an empty effect row and no implicit environment".into());
        }
        let scalar = |term: &Term| {
            matches!(
                term,
                Term::Pointer(_)
                    | Term::Primitive(
                        P::Bool
                            | P::U8
                            | P::U16
                            | P::U32
                            | P::U64
                            | P::I8
                            | P::I16
                            | P::I32
                            | P::I64
                            | P::Isize
                            | P::Usize
                            | P::F32
                            | P::F64
                    )
            )
        };
        let never = *result == Term::Primitive(P::Never);
        if !scalar(result) && !matches!(result, Term::Primitive(P::Void | P::Never)) {
            return Err("unsupported foreign ABI result representation".into());
        }
        let result = return_repr(graph, result, true)?;
        let name = &function.name;
        let published = names.get_or_insert_with(|| {
            module
                .lines()
                .filter(|line| {
                    line.starts_with("define ")
                        || line.starts_with("declare ")
                        || line.starts_with('@')
                })
                .filter_map(|line| line.split_once('@'))
                .map(|(_, tail)| tail.split(['(', ' ', '=']).next().unwrap().to_owned())
                .collect::<BTreeSet<_>>()
        });
        if !published.insert(name.clone()) {
            return Err(format!("conflicting host foreign ABI symbol {name}"));
        }
        let extension = |ty: &Ty| match ty {
            Ty::Bool | Ty::Int(8 | 16, false) => "zeroext ",
            Ty::Int(8 | 16, true) => "signext ",
            _ => "",
        };
        let mut inputs = Vec::new();
        let mut parameters_c = Vec::new();
        for (i, parameter) in parameters.iter().enumerate() {
            if parameter.sort != Sort::Runtime || !scalar(&parameter.ty) {
                return Err("unsupported foreign ABI parameter representation".into());
            }
            let ty = repr(graph, &parameter.ty, true)?;
            inputs.push(format!("{} %arg{i}", ty.llvm()));
            parameters_c.push(format!("{} {}%arg{i}", ty.llvm(), extension(&ty)));
        }
        let decl = parameters_c.join(", ");
        inputs.push("ptr %task".into());
        let args = inputs.join(", ");
        let packet = packet(&result).llvm();
        let completed = if never {
            "unreachable".into()
        } else if result == Ty::Unit {
            "ret void".into()
        } else {
            format!(
                "%value = extractvalue {packet} %r, 0\n  ret {} %value",
                result.llvm()
            )
        };
        let track_c_frame = module.contains("@flurry_c_frame_depth");
        let cc = super::abi_calling_convention_for_triple(convention, graph.target, target_triple);
        let symbol: String = name.bytes().map(|b| format!("\\{b:02X}")).collect();
        let c_depth_enter = if track_c_frame {
            "  %c_depth_0 = load i64, ptr @flurry_c_frame_depth, align 8\n  %c_depth_1 = add i64 %c_depth_0, 1\n  store i64 %c_depth_1, ptr @flurry_c_frame_depth, align 8\n"
        } else {
            ""
        };
        let c_depth_exit = if track_c_frame {
            "  store i64 %c_depth_0, ptr @flurry_c_frame_depth, align 8\n"
        } else {
            ""
        };
        module.push_str(&format!("define {cc}{}{} @\"{symbol}\"({decl}) {{\nentry:\n  %task = alloca %flurry.task\n  call void @flurry_task_init(ptr %task, ptr null)\n{c_depth_enter}  %r = call {packet} @flurry_{index}({args})\n{c_depth_exit}  %ctrl = extractvalue {packet} %r, 1\n  %normal = icmp eq ptr %ctrl, null\n  br i1 %normal, label %done, label %escape, {LIKELY}\nescape:\n  call void @llvm.trap()\n  unreachable\ndone:\n  {completed}\n}}\n", extension(&result), result.llvm()));
    }
    Ok(())
}
