//! Closed error rows are tagged return data, without effect continuations.
use super::*;
use nir::error_results::Conversion;

impl Emitter<'_> {
    pub(super) fn error_index(&self, ty: &Ty, label: Option<&Term>) -> Result<usize, String> {
        let Ty::ErrorResult(layout) = ty else {
            return Err("host error result layout missing".into());
        };
        layout
            .branches
            .iter()
            .position(|(candidate, _)| candidate.as_ref() == label)
            .ok_or("host error label missing".into())
    }

    pub(super) fn error_tag(&mut self, value: &Value) -> Result<Value, String> {
        let Ty::ErrorResult(layout) = &value.ty else {
            return Err("host error result tag requires its layout".into());
        };
        Ok(self.instruction(
            Ty::Int(layout.tag_bits, false),
            format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
        ))
    }

    pub(super) fn error_payload(&mut self, value: &Value, index: usize) -> Result<Value, String> {
        let Ty::ErrorResult(layout) = &value.ty else {
            return Err("host error payload layout missing".into());
        };
        let branch_ty = layout
            .branches
            .get(index)
            .ok_or("host error tag out of range")?
            .1
            .clone();
        if branch_ty == Ty::Unit {
            return Ok(Value::unit());
        }
        if layout.payload_size == 0 {
            return Ok(Value {
                ty: branch_ty,
                text: "zeroinitializer".into(),
            });
        }
        let slot = self.slot(value.ty.clone());
        self.store(&slot, value)?;
        let payload_ptr = self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr {}, ptr {}, i32 0, i32 1",
                value.ty.llvm(),
                slot.pointer
            ),
        );
        let payload_slot = Slot::new(
            branch_ty.clone(),
            payload_ptr.text,
            matches!(branch_ty, Ty::PackedRecord(_)).then_some(1),
        );
        Ok(self.load(&payload_slot))
    }

    fn error_pack(&mut self, ty: Ty, index: usize, payload: Value) -> Result<Value, String> {
        let Ty::ErrorResult(layout) = &ty else {
            return Err("host error destination layout missing".into());
        };
        if layout.branches.get(index).map(|(_, ty)| ty) != Some(&payload.ty) {
            return Err("host error payload representation mismatch".into());
        }
        let tag_bits = layout.tag_bits;
        if payload.ty == Ty::Unit || layout.payload_size == 0 {
            return Ok(self.instruction(
                ty.clone(),
                format!(
                    "insertvalue {} zeroinitializer, i{tag_bits} {index}, 0",
                    ty.llvm()
                ),
            ));
        }
        let slot = self.slot(ty.clone());
        self.store(
            &slot,
            &Value {
                ty: ty.clone(),
                text: "zeroinitializer".into(),
            },
        )?;
        let tag_ptr = self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr {}, ptr {}, i32 0, i32 0",
                ty.llvm(),
                slot.pointer
            ),
        );
        self.line(format!("store i{tag_bits} {index}, ptr {}", tag_ptr.text));
        let payload_ptr = self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr {}, ptr {}, i32 0, i32 1",
                ty.llvm(),
                slot.pointer
            ),
        );
        let payload_slot = Slot::new(
            payload.ty.clone(),
            payload_ptr.text,
            matches!(payload.ty, Ty::PackedRecord(_)).then_some(1),
        );
        self.store(&payload_slot, &payload)?;
        Ok(self.load(&slot))
    }

    pub(super) fn error_convert(
        &mut self,
        conversion: Conversion,
        value: Value,
        source: &Term,
        target: &Term,
    ) -> Result<Value, String> {
        let ty = repr(self.graph, target, self.checked)?;
        match conversion {
            Conversion::Success | Conversion::Failure => {
                let label = if conversion == Conversion::Failure {
                    Some(source)
                } else {
                    None
                };
                let index = self.error_index(&ty, label)?;
                self.error_pack(ty, index, value)
            }
            Conversion::Embed => {
                let Ty::ErrorResult(layout) = &value.ty else {
                    return Err("host error embedding source layout missing".into());
                };
                let fields = layout.branches.clone();
                let src_tag_bits = layout.tag_bits;
                let tag = self.error_tag(&value)?;
                let result = self.slot(ty.clone());
                let done = self.fresh();
                for (index, (label, _)) in fields.iter().enumerate() {
                    if index == 0
                        && matches!(source, Term::ErrorResult { result, .. } if **result == Term::Primitive(P::Never))
                    {
                        continue;
                    }
                    let yes = self.fresh();
                    let next = self.fresh();
                    let condition = self.instruction(
                        Ty::Bool,
                        format!("icmp eq i{src_tag_bits} {}, {index}", tag.text),
                    );
                    self.line(format!(
                        "br i1 {}, label %{yes}, label %{next}",
                        condition.text
                    ));
                    self.label(&yes);
                    let payload = self.error_payload(&value, index)?;
                    let destination = self.error_index(&ty, label.as_ref())?;
                    let packed = self.error_pack(ty.clone(), destination, payload)?;
                    self.store(&result, &packed)?;
                    self.branch(&done);
                    self.label(&next);
                }
                self.emit_trap();
                self.line("unreachable");
                self.label(&done);
                Ok(self.load(&result))
            }
            Conversion::Product => {
                let graph = self.graph;
                let plan = nir::error_results::transport_plan_with_layouts(
                    source,
                    target,
                    true,
                    self.fuel,
                    &mut |a, b| a == b,
                    &|ty| {
                        graph
                            .normal_forms
                            .iter()
                            .find_map(|head| head.tuple_elements(ty))
                    },
                )
                .map_err(|e| format!("host error product transport: {e}"))?;
                self.error_transport(&plan, value, ty, 0)
            }
        }
    }
}

impl Emitter<'_> {
    fn error_transport(
        &mut self,
        plan: &nir::error_results::RepresentationTransport,
        value: Value,
        target: Ty,
        depth: usize,
    ) -> Result<Value, String> {
        use nir::error_results::RepresentationTransport as T;
        if depth >= 128 || *self.fuel == 0 {
            return Err("host error transport budget exhausted".into());
        }
        *self.fuel -= 1;
        match plan {
            T::Identity => {
                if value.ty != target {
                    return Err("host identity transport changed layout".into());
                }
                Ok(value)
            }
            T::Error {
                source,
                target: destination,
                ..
            } => {
                let result = self.error_convert(Conversion::Embed, value, source, destination)?;
                if result.ty != target {
                    return Err("host error transport destination mismatch".into());
                }
                Ok(result)
            }
            T::Tuple(plans) => self.error_transport_fields(plans.iter(), value, target, depth),
            T::Record(plans) => self.error_transport_fields(
                plans.iter().map(|(_, plan)| plan),
                value,
                target,
                depth,
            ),
        }
    }

    fn error_transport_fields<'a>(
        &mut self,
        plans: impl ExactSizeIterator<Item = &'a nir::error_results::RepresentationTransport>,
        value: Value,
        target: Ty,
        depth: usize,
    ) -> Result<Value, String> {
        if plans.len() == 0 && value.ty == Ty::Unit && target == Ty::Unit {
            return Ok(value);
        }
        let (
            Ty::Record(source_fields) | Ty::PackedRecord(source_fields),
            Ty::Record(target_fields) | Ty::PackedRecord(target_fields),
        ) = (&value.ty, &target)
        else {
            return Err("host product transport requires product layouts".into());
        };
        if plans.len() != target_fields.len() || source_fields.len() != target_fields.len() {
            return Err("host product transport field count mismatch".into());
        }
        let target_fields = target_fields.clone();
        let mut result = Value {
            ty: target.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, (plan, field_ty)) in plans.zip(target_fields.iter()).enumerate() {
            let input = self.record_field(&value, index)?;
            let field = self.error_transport(plan, input, field_ty.clone(), depth + 1)?;
            let (storage, text) = if field.ty == Ty::Unit {
                ("{}".into(), "zeroinitializer".into())
            } else {
                (field.ty.llvm(), field.text)
            };
            result = self.instruction(
                target.clone(),
                format!(
                    "insertvalue {} {}, {storage} {text}, {index}",
                    target.llvm(),
                    result.text
                ),
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_only_success_placeholder_does_not_make_never_storable() {
        let graph = iir::Package::default();
        let never = Term::Primitive(P::Never);
        assert!(repr(&graph, &never, true).is_err());
        assert!(
            repr(
                &graph,
                &Term::Record(vec![("value".into(), never.clone())]),
                true
            )
            .is_err()
        );
        let error_only = Term::ErrorResult {
            errors: vec![Term::Primitive(P::Bool)],
            result: Box::new(never),
        };
        assert!(matches!(
            repr(&graph, &error_only, true).unwrap(),
            Ty::ErrorResult(_)
        ));
    }

    #[test]
    fn native_product_conversion_reorders_nested_error_rows() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let row = |errors| Term::ErrorResult {
            errors,
            result: Box::new(Term::Primitive(P::Bool)),
        };
        let source_row = row(vec![Term::Primitive(P::Bool), Term::Primitive(P::Char)]);
        let target_row = row(vec![Term::Primitive(P::Char), Term::Primitive(P::Bool)]);
        let wrap = |row| {
            Term::Record(vec![
                ("inner".into(), Term::Record(vec![("result".into(), row)])),
                ("sentinel".into(), Term::Primitive(P::Bool)),
            ])
        };
        let source = wrap(source_row.clone());
        let target = wrap(target_row.clone());
        for tag in [0, 1, 2, 9] {
            let mut declarations = BTreeSet::new();
            let mut fuel = 10_000;
            let mut emitter = Emitter {
                graph: &graph,
                cleanup: None,
                schedules: None,
                resource_unwinds: vec![],
                checked: true,
                function: None,
                closure_body: None,
                failure_result: None,
                effects: vec![],
                effect_resume: None,
                native_return: None,
                loop_destinations: Default::default(),
                outlined_return: None,
                outlined_loops: BTreeMap::new(),
                lambdas: &lambdas,
                template_identities: &template_identities,
                locals: BTreeMap::new(),
                live: BTreeMap::new(),
                temporaries: BTreeMap::new(),
                temporary_live: BTreeMap::new(),
                prepared: BTreeMap::new(),
                receipt_paths: vec![],
                pattern_orders: BTreeMap::new(),
                temporary_fields: BTreeMap::new(),
                loops: BTreeMap::new(),
                prologue: String::new(),
                body: String::new(),
                emitted_effects: attributes::EmittedBodyEffects::default(),
                declarations: &mut declarations,
                text_constants: &mut strings::TextConstants::default(),
                attr_table: None,
                bitvec_cache: None,
                serial: 0,
                terminated: false,
                fuel: &mut fuel,
            };
            let row_ty = repr(&graph, &source_row, true).unwrap();
            let Ty::ErrorResult(row_layout) = &row_ty else {
                panic!("expected error result layout");
            };
            let packed = if tag == 9 {
                emitter.instruction(
                    row_ty.clone(),
                    format!(
                        "insertvalue {} zeroinitializer, i{} 9, 0",
                        row_ty.llvm(),
                        row_layout.tag_bits
                    ),
                )
            } else {
                let payload = if tag == 2 {
                    Value {
                        ty: Ty::Int(32, false),
                        text: "65".into(),
                    }
                } else {
                    Value {
                        ty: Ty::Bool,
                        text: "true".into(),
                    }
                };
                emitter.error_pack(row_ty, tag, payload).unwrap()
            };
            let source_ty = repr(&graph, &source, true).unwrap();
            let nested = emitter.instruction(
                source_ty.clone(),
                format!(
                    "insertvalue {} zeroinitializer, {} {}, 0, 0",
                    source_ty.llvm(),
                    packed.ty.llvm(),
                    packed.text,
                ),
            );
            let input = emitter.instruction(
                source_ty.clone(),
                format!(
                    "insertvalue {} {}, i1 true, 1",
                    source_ty.llvm(),
                    nested.text,
                ),
            );
            // Exercise the backend Product entry directly: source elaboration
            // may canonicalize both rows and eliminate this representation edge.
            let converted = emitter
                .error_convert(Conversion::Product, input, &source, &target)
                .unwrap();
            let sentinel = emitter.record_field(&converted, 1).unwrap();
            emitter.guard(&sentinel).unwrap();
            let nested = emitter.record_field(&converted, 0).unwrap();
            let result = emitter.record_field(&nested, 0).unwrap();
            let expected = match tag {
                0 => 0,
                1 => 2,
                _ => 1,
            };
            let actual = emitter.error_tag(&result).unwrap();
            let Ty::ErrorResult(result_layout) = &result.ty else {
                panic!("expected converted error result layout");
            };
            let equal = emitter.instruction(
                Ty::Bool,
                format!(
                    "icmp eq i{} {}, {expected}",
                    result_layout.tag_bits, actual.text
                ),
            );
            emitter.guard(&equal).unwrap();
            let payload = emitter.error_payload(&result, expected).unwrap();
            let equal = emitter.instruction(
                Ty::Bool,
                format!(
                    "icmp eq {} {}, {}",
                    payload.ty.llvm(),
                    payload.text,
                    if expected == 1 { "65" } else { "true" },
                ),
            );
            emitter.guard(&equal).unwrap();
            emitter.line("ret i32 0");
            let ir = format!(
                "declare void @llvm.trap()\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
                emitter.prologue, emitter.body
            );
            for optimization in [
                crate::host::OptimizationLevel::O0,
                crate::host::OptimizationLevel::O3,
            ] {
                let toolchain = crate::host::Toolchain {
                    optimization,
                    ..Default::default()
                };
                let output = toolchain.execute_test_ir(&ir).unwrap();
                assert_eq!(output.status.success(), tag != 9, "tag {tag}");
            }
        }
    }

    #[test]
    fn sum_type_abi_reduces_error_row_size_and_aligns_llvm_union() {
        let graph = iir::Package::default();
        let u8_u64 = Term::ErrorResult {
            errors: vec![Term::Primitive(P::U8)],
            result: Box::new(Term::Primitive(P::U64)),
        };
        let u8_u64_ty = repr(&graph, &u8_u64, true).unwrap();
        assert_eq!(u8_u64_ty.llvm(), "{ i8, [1 x i64] }");
        assert_eq!(
            u8_u64_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
            nir::TypeLayout { size: 16, align: 8 }
        );

        let many_u64 = Term::ErrorResult {
            errors: vec![Term::Primitive(P::U64); 4],
            result: Box::new(Term::Primitive(P::U64)),
        };
        let many_u64_ty = repr(&graph, &many_u64, true).unwrap();
        assert_eq!(many_u64_ty.llvm(), "{ i8, [1 x i64] }");
        assert_eq!(
            many_u64_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
            nir::TypeLayout { size: 16, align: 8 }
        );

        let u128_u8 = Term::ErrorResult {
            errors: vec![Term::Primitive(P::U128)],
            result: Box::new(Term::Primitive(P::U8)),
        };
        let u128_u8_ty = repr(&graph, &u128_u8, true).unwrap();
        assert_eq!(u128_u8_ty.llvm(), "{ i8, [1 x i128] }");
        assert_eq!(
            u128_u8_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
            nir::TypeLayout {
                size: 32,
                align: 16,
            }
        );

        let void_void = Term::ErrorResult {
            errors: vec![Term::Primitive(P::Void)],
            result: Box::new(Term::Primitive(P::Void)),
        };
        let void_void_ty = repr(&graph, &void_void, true).unwrap();
        assert_eq!(void_void_ty.llvm(), "{ i8, {} }");
        assert_eq!(
            void_void_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
            nir::TypeLayout { size: 1, align: 1 }
        );

        let packed_9b_layout = Ty::ErrorResult(std::sync::Arc::new(
            ErrorResultLayout::new(
                vec![
                    (None, Ty::Int(64, false)),
                    (
                        Some(Term::Primitive(P::U8)),
                        Ty::PackedRecord(std::sync::Arc::new(vec![
                            Ty::Int(64, false),
                            Ty::Int(8, false),
                        ])),
                    ),
                ],
                64,
            )
            .unwrap(),
        ));
        assert_eq!(packed_9b_layout.llvm(), "{ i8, [2 x i64] }");
        assert_eq!(
            packed_9b_layout
                .storage_layout_at(64, 0, &mut 1000)
                .unwrap(),
            nir::TypeLayout { size: 24, align: 8 }
        );
    }

    #[test]
    fn sum_type_abi_o3_eliminates_alloca_roundtrip() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let row = Term::ErrorResult {
            errors: vec![Term::Primitive(P::U8)],
            result: Box::new(Term::Primitive(P::U64)),
        };
        let row_ty = repr(&graph, &row, true).unwrap();
        let mut declarations = BTreeSet::new();
        let mut fuel = 10_000;
        let mut emitter = Emitter {
            graph: &graph,
            cleanup: None,
            schedules: None,
            resource_unwinds: vec![],
            checked: true,
            function: None,
            closure_body: None,
            failure_result: None,
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: &lambdas,
            template_identities: &template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            emitted_effects: attributes::EmittedBodyEffects::default(),
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            bitvec_cache: None,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };
        let packed = emitter
            .error_pack(
                row_ty.clone(),
                0,
                Value {
                    ty: Ty::Int(64, false),
                    text: "%x".into(),
                },
            )
            .unwrap();
        let tag = emitter.error_tag(&packed).unwrap();
        let payload = emitter.error_payload(&packed, 0).unwrap();
        let unpacked = emitter
            .error_payload(
                &Value {
                    ty: row_ty.clone(),
                    text: "%v".into(),
                },
                1,
            )
            .unwrap();
        emitter.line(format!("ret {} {}", payload.ty.llvm(), payload.text));
        let ir = format!(
            "define {} @roundtrip(i64 %x, {} %v) {{\nentry:\n{}{}}}\n",
            payload.ty.llvm(),
            row_ty.llvm(),
            emitter.prologue,
            emitter.body
        );
        let _ = (tag, unpacked);
        let toolchain = crate::host::Toolchain {
            optimization: crate::host::OptimizationLevel::O3,
            ..Default::default()
        };
        let optimized = toolchain.optimized_ir(&ir).unwrap();
        assert!(!optimized.contains("alloca"), "{optimized}");
        assert!(optimized.contains("ret i64 %x"), "{optimized}");
    }
}
