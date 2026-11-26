//! Privileged list primitives used by the ordinary meta package wrappers.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn object_edit_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        use n::ObjectEditKind as O;
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(expr.span, "Object edits require comptime invocation"));
        }
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        let operation = match name.as_str() {
            "object_concat" => O::Concat,
            "object_append" => O::Append,
            "object_replace" => O::Replace,
            "object_remove" => O::Remove,
            "object_slice" => O::Slice,
            _ => unreachable!(),
        };
        let count = if matches!(operation, O::Append | O::Replace | O::Slice) {
            3
        } else {
            2
        };
        if args.len() != count || args.iter().any(|arg| !matches!(arg, Arg::Positional(_))) {
            return Err(error(
                expr.span,
                "Object edit requires positional Object and edit operands",
            ));
        }
        let mut fields = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let Arg::Positional(value) = arg else {
                unreachable!()
            };
            let domain = match index {
                0 => Some(p(P::Object)),
                1 => Some(match operation {
                    O::Concat => p(P::Object),
                    O::Append => Term::Optional(Box::new(p(P::Symbol))),
                    _ => p(P::Integer),
                }),
                2 if operation == O::Slice => Some(p(P::Integer)),
                _ => None,
            };
            fields.push(self.expr(value, domain.as_ref())?);
        }
        let ty = if fields.iter().any(|field| is_never(&field.ty)) {
            p(P::Never)
        } else {
            p(P::Object)
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ObjectEdit {
                operation,
                subject: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect()),
                    kind: K::Tuple(fields),
                }),
            },
        })
    }

    pub(super) fn object_query_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        use n::ObjectQueryKind as Q;
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "Object queries require comptime invocation",
            ));
        }
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        let query = match name.as_str() {
            "object_length" => Q::Length,
            "object_key" => Q::Key,
            "object_entry_type" => Q::EntryType,
            "object_value" => Q::Value,
            "object_get" => Q::NamedValue,
            _ => unreachable!(),
        };
        let count = match query {
            Q::Length => 1,
            Q::Value | Q::NamedValue => 3,
            _ => 2,
        };
        if args.len() != count || args.iter().any(|arg| !matches!(arg, Arg::Positional(_))) {
            return Err(error(
                expr.span,
                "Object query requires positional inputs: optional target Type, Object, and index or key",
            ));
        }
        let (target, args) = if matches!(query, Q::Value | Q::NamedValue) {
            let Arg::Positional(target) = &args[0] else {
                unreachable!()
            };
            (Some(self.type_expr(target)?), &args[1..])
        } else {
            (None, args)
        };
        let mut fields = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let Arg::Positional(value) = arg else {
                unreachable!()
            };
            fields.push(self.expr(
                value,
                Some(&p(if index == 0 {
                    P::Object
                } else if query == Q::NamedValue {
                    P::Symbol
                } else {
                    P::Integer
                })),
            )?);
        }
        let result = match query {
            Q::Length => p(P::Integer),
            Q::Key => Term::Optional(Box::new(p(P::Symbol))),
            Q::EntryType => p(P::Type),
            Q::Value | Q::NamedValue => Term::Optional(Box::new(target.unwrap())),
        };
        let ty = if fields.iter().any(|field| is_never(&field.ty)) {
            p(P::Never)
        } else {
            result
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        let packet = n::Expr {
            source: source(expr.span),
            ty: Term::Tuple(fields.iter().map(|field| field.ty.clone()).collect()),
            kind: K::Tuple(fields),
        };
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ObjectQuery {
                subject: Box::new(packet),
                query,
            },
        })
    }

    fn ensure_meta_list_metadata(&mut self, head: DeclarationId, span: Span) -> Result<(), Error> {
        let owner = self
            .source_declarations
            .owner(head)
            .ok_or_else(|| error(span, "bound meta.List declaration is unavailable"))?;
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        self.refresh_list_inference_metadata();
        Ok(())
    }

    /// Constant initializers can discover a library signature after their
    /// lexical context was created. Refresh only matching metadata; never
    /// import another declaration's local assumptions or formation facts.
    pub(super) fn refresh_list_inference_metadata(&mut self) {
        let Some(head) = self.protocols.meta_list else {
            return;
        };
        if self.context.is_list_inference_head(head) {
            return;
        }
        if let Some(owner) = self.source_declarations.owner(head)
            && self
                .normal_head_contract(owner)
                .is_some_and(|(_, role)| role == nir::normal_forms::Role::TypeList)
        {
            self.context.admit_list_inference_head(head);
        }
    }

    pub(super) fn newtype_reflection_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let [Arg::Positional(subject)] = args else {
            return Err(error(
                expr.span,
                "newtype reflection requires one positional Type input",
            ));
        };
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "newtype reflection requires comptime invocation",
            ));
        }
        let subject = self.expr(subject, Some(&p(P::Type)))?;
        let ty = if is_never(&subject.ty) {
            p(P::Never)
        } else {
            p(P::Type)
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::NewtypeRepresentation(Box::new(subject)),
        })
    }

    pub(super) fn struct_reflection_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let [Arg::Positional(subject)] = args else {
            return Err(error(
                expr.span,
                "struct reflection requires one positional Type input",
            ));
        };
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "struct reflection requires comptime invocation",
            ));
        }
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        let head = self
            .protocols
            .meta_list
            .ok_or_else(|| error(expr.span, "struct reflection requires a bound meta.List<T>"))?;
        self.ensure_meta_list_metadata(head, expr.span)?;
        let subject = self.expr(subject, Some(&p(P::Type)))?;
        let (element, kind) = match name.as_str() {
            "struct_field_names" => (p(P::Symbol), K::StructFieldNames(Box::new(subject))),
            "struct_field_types" => (p(P::Type), K::StructFieldTypes(Box::new(subject))),
            "record_field_names" => (p(P::Symbol), K::RecordFieldNames(Box::new(subject))),
            "record_field_types" => (p(P::Type), K::RecordFieldTypes(Box::new(subject))),
            "struct_field_spans" | "record_field_spans" => {
                let span_decl = self.protocols.source_span.ok_or_else(|| {
                    error(
                        expr.span,
                        "span reflection requires a bound SourceSpan type",
                    )
                })?;
                let span_ty = Term::Nominal {
                    declaration: span_decl,
                    arguments: vec![],
                };
                (
                    span_ty,
                    if name.as_str() == "struct_field_spans" {
                        K::StructFieldSpans(Box::new(subject))
                    } else {
                        K::RecordFieldSpans(Box::new(subject))
                    },
                )
            }
            _ => unreachable!(),
        };
        let ty = if is_never(match &kind {
            K::StructFieldNames(s)
            | K::StructFieldTypes(s)
            | K::RecordFieldNames(s)
            | K::RecordFieldTypes(s)
            | K::StructFieldSpans(s)
            | K::RecordFieldSpans(s) => &s.ty,
            _ => unreachable!(),
        }) {
            p(P::Never)
        } else {
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![element],
            }
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind,
        })
    }

    pub(super) fn span_reflection_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let [Arg::Positional(subject)] = args else {
            return Err(error(
                expr.span,
                "declaration_span requires one positional Type input",
            ));
        };
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "declaration_span requires comptime invocation",
            ));
        }
        let span_decl = self.protocols.source_span.ok_or_else(|| {
            error(
                expr.span,
                "declaration_span requires a bound SourceSpan type",
            )
        })?;
        let subject = self.expr(subject, Some(&p(P::Type)))?;
        let ty = if is_never(&subject.ty) {
            p(P::Never)
        } else {
            Term::Nominal {
                declaration: span_decl,
                arguments: vec![],
            }
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::DeclarationSpan(Box::new(subject)),
        })
    }

    pub(super) fn enum_reflection_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "enum reflection requires comptime invocation",
            ));
        }
        let [Arg::Positional(subject)] = args else {
            return Err(error(
                expr.span,
                "enum reflection requires one positional Type input",
            ));
        };
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        if matches!(
            name.as_str(),
            "enum_variant_spans" | "enum_variant_field_spans"
        ) {
            let head = self
                .protocols
                .meta_list
                .ok_or_else(|| error(expr.span, "enum reflection requires a bound meta.List<T>"))?;
            self.ensure_meta_list_metadata(head, expr.span)?;
            let span_decl = self.protocols.source_span.ok_or_else(|| {
                error(
                    expr.span,
                    "enum span reflection requires a bound SourceSpan type",
                )
            })?;
            let span_ty = Term::Nominal {
                declaration: span_decl,
                arguments: vec![],
            };
            let subject = self.expr(subject, Some(&p(P::Type)))?;
            let list = |element| Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![element],
            };
            let nested = name.as_str() == "enum_variant_field_spans";
            let ty = if is_never(&subject.ty) {
                p(P::Never)
            } else if nested {
                list(list(span_ty))
            } else {
                list(span_ty)
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, expr.span)?;
            }
            return Ok(n::Expr {
                source: source(expr.span),
                ty,
                kind: if nested {
                    K::EnumVariantFieldSpans(Box::new(subject))
                } else {
                    K::EnumVariantSpans(Box::new(subject))
                },
            });
        }
        use n::{EnumBitQuery as B, EnumStructureQuery as Q};
        let query = match name.as_str() {
            "enum_group_paths" => Q::GroupPaths,
            "enum_variant_paths" => Q::VariantPaths,
            "enum_variant_shapes" => Q::Shapes,
            "enum_variant_field_names" => Q::FieldNames,
            "enum_variant_field_types" => Q::FieldTypes,
            "enum_symbol_values" => Q::SymbolValues,
            "enum_bit_width" => Q::BitEncoding(B::Width),
            "enum_bit_base_type" => Q::BitEncoding(B::BaseType),
            "enum_bit_masks" => Q::BitEncoding(B::Masks),
            "enum_bit_values" => Q::BitEncoding(B::Values),
            "enum_bit_offsets" => Q::BitEncoding(B::Offsets),
            _ => unreachable!(),
        };
        let head = self
            .protocols
            .meta_list
            .ok_or_else(|| error(expr.span, "enum reflection requires a bound meta.List<T>"))?;
        self.ensure_meta_list_metadata(head, expr.span)?;
        let subject = self.expr(subject, Some(&p(P::Type)))?;
        let list = |element| Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![element],
        };
        let element = p(if query == Q::FieldTypes {
            P::Type
        } else {
            P::Symbol
        });
        let ty = if is_never(&subject.ty) {
            p(P::Never)
        } else if let Q::BitEncoding(query) = query {
            match query {
                B::Width => p(P::Integer),
                B::BaseType => Term::Optional(Box::new(p(P::Type))),
                B::Masks | B::Values => list(p(P::Integer)),
                B::Offsets => list(list(p(P::Integer))),
            }
        } else if matches!(query, Q::Shapes | Q::SymbolValues) {
            list(element)
        } else {
            list(list(element))
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::EnumStructure {
                subject: Box::new(subject),
                query,
            },
        })
    }

    pub(super) fn build_enum_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        segmented_paths: bool,
        symbol_encoding: bool,
        fixed_encoding: bool,
    ) -> Result<n::Expr, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "enum construction requires comptime invocation",
            ));
        }
        let (args, bit_args) = if fixed_encoding {
            if args.len() != 11 {
                return Err(error(
                    expr.span,
                    "bit enum construction requires descriptor, width, base type, masks, values and offsets",
                ));
            }
            let (descriptor, encoding) = args.split_at(6);
            (descriptor, Some(encoding))
        } else {
            (args, None)
        };
        let (args, codes) = if symbol_encoding {
            let Some((Arg::Positional(codes), args)) = args.split_last() else {
                return Err(error(
                    expr.span,
                    "Symbol enum construction requires an encoding list",
                ));
            };
            (args, Some(codes))
        } else {
            (args, None)
        };
        let [
            Arg::Positional(name),
            Arg::Positional(groups),
            Arg::Positional(names),
            Arg::Positional(shapes),
            Arg::Positional(field_names),
            Arg::Positional(field_types),
        ] = args
        else {
            return Err(error(
                expr.span,
                "enum construction requires name, groups, variant names, shapes, field names and field Types",
            ));
        };
        let head = self
            .protocols
            .meta_list
            .ok_or_else(|| error(expr.span, "enum construction requires a bound meta.List<T>"))?;
        self.ensure_meta_list_metadata(head, expr.span)?;
        let list = |element| Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![element],
        };
        let paths = if segmented_paths {
            list(list(p(P::Symbol)))
        } else {
            list(p(P::Symbol))
        };
        let mut values = vec![
            self.expr(name, Some(&p(P::Symbol)))?,
            self.expr(groups, Some(&paths))?,
            self.expr(names, Some(&paths))?,
            self.expr(shapes, Some(&list(p(P::Symbol))))?,
            self.expr(field_names, Some(&list(list(p(P::Symbol)))))?,
            self.expr(field_types, Some(&list(list(p(P::Type)))))?,
        ];
        if let Some(codes) = codes {
            values.push(self.expr(codes, Some(&list(p(P::Symbol))))?);
        }
        if let Some(bit_args) = bit_args {
            let [
                Arg::Positional(width),
                Arg::Positional(base),
                Arg::Positional(masks),
                Arg::Positional(fixed),
                Arg::Positional(offsets),
            ] = bit_args
            else {
                return Err(error(
                    expr.span,
                    "bit enum construction requires positional encoding inputs",
                ));
            };
            let parts = vec![
                self.expr(width, Some(&p(P::Integer)))?,
                self.expr(base, Some(&Term::Optional(Box::new(p(P::Type)))))?,
                self.expr(masks, Some(&list(p(P::Integer))))?,
                self.expr(fixed, Some(&list(p(P::Integer))))?,
                self.expr(offsets, Some(&list(list(p(P::Integer)))))?,
            ];
            values.push(n::Expr {
                source: source(expr.span),
                ty: if parts.iter().any(|part| is_never(&part.ty)) {
                    p(P::Never)
                } else {
                    Term::Tuple(parts.iter().map(|part| part.ty.clone()).collect())
                },
                kind: K::Tuple(parts),
            });
        }
        let ty = p(if values.iter().any(|value| is_never(&value.ty)) {
            P::Never
        } else {
            P::Type
        });
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::BuildEnum(Box::new(n::Expr {
                source: source(expr.span),
                ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
                kind: K::Tuple(values),
            })),
        })
    }

    pub(super) fn build_newtype_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "newtype construction requires comptime invocation",
            ));
        }
        let [Arg::Positional(name), Arg::Positional(representation)] = args else {
            return Err(error(
                expr.span,
                "newtype construction requires name and representation Type",
            ));
        };
        let values = vec![
            self.expr(name, Some(&p(P::Symbol)))?,
            self.expr(representation, Some(&p(P::Type)))?,
        ];
        let ty = p(if values.iter().any(|value| is_never(&value.ty)) {
            P::Never
        } else {
            P::Type
        });
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::BuildNewtype(Box::new(n::Expr {
                source: source(expr.span),
                ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
                kind: K::Tuple(values),
            })),
        })
    }

    pub(super) fn build_struct_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "struct construction requires comptime invocation",
            ));
        }
        let [
            Arg::Positional(name),
            Arg::Positional(names),
            Arg::Positional(types),
        ] = args
        else {
            return Err(error(
                expr.span,
                "struct construction requires name, field names and field Types",
            ));
        };
        let head = self.protocols.meta_list.ok_or_else(|| {
            error(
                expr.span,
                "struct construction requires a bound meta.List<T>",
            )
        })?;
        self.ensure_meta_list_metadata(head, expr.span)?;
        let list = |element| Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![p(element)],
        };
        let values = vec![
            self.expr(name, Some(&p(P::Symbol)))?,
            self.expr(names, Some(&list(P::Symbol)))?,
            self.expr(types, Some(&list(P::Type)))?,
        ];
        let ty = p(if values.iter().any(|value| is_never(&value.ty)) {
            P::Never
        } else {
            P::Type
        });
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::BuildStruct(Box::new(n::Expr {
                source: source(expr.span),
                ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
                kind: K::Tuple(values),
            })),
        })
    }

    pub(super) fn list_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        let length = name.as_str() == "list_length";
        if args.len() != if length { 1 } else { 2 }
            || args.iter().any(|arg| !matches!(arg, Arg::Positional(_)))
        {
            return Err(error(
                expr.span,
                "list primitive requires positional list operands",
            ));
        }
        let Arg::Positional(first) = &args[0] else {
            unreachable!()
        };
        let left = self.expr(first, if length { None } else { expected })?;
        if self.list_element_type(&left.ty).is_none() && !is_never(&left.ty) {
            return Err(error(
                first.span,
                "list primitive requires a bound meta.List<T>",
            ));
        }
        let (ty, kind) = if length {
            (
                if is_never(&left.ty) {
                    p(P::Never)
                } else {
                    p(P::Integer)
                },
                K::ListLength(Box::new(left)),
            )
        } else {
            let Arg::Positional(second) = &args[1] else {
                unreachable!()
            };
            let right = self.expr(second, (!is_never(&left.ty)).then_some(&left.ty))?;
            if self.list_element_type(&right.ty).is_none() && !is_never(&right.ty) {
                return Err(error(
                    second.span,
                    "list primitive requires a bound meta.List<T>",
                ));
            }
            let ty = if is_never(&left.ty) || is_never(&right.ty) {
                p(P::Never)
            } else {
                left.ty.clone()
            };
            (ty, K::ListConcat(Box::new(left), Box::new(right)))
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind,
        })
    }

    pub(super) fn resolve_tuple_append_type(&self, tuple: Term, element: Term) -> Term {
        match tuple {
            Term::Primitive(P::Void | P::Unit) | Term::Unit => Term::Tuple(vec![element]),
            Term::Tuple(mut items) => {
                items.push(element);
                Term::Tuple(items)
            }
            other => {
                if let Some(items) = self.bound_tuple_elements(&other) {
                    let mut extended = items.to_vec();
                    extended.push(element);
                    Term::Tuple(extended)
                } else {
                    Term::tuple_append(other, element)
                }
            }
        }
    }

    pub(super) fn reflection_op_builtin(
        &mut self,
        expr: &Expr<'_>,
        name: &str,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let mut positional = Vec::with_capacity(args.len());
        for arg in args {
            let Arg::Positional(value) = arg else {
                return Err(error(
                    expr.span,
                    format!("{name} builtin requires positional arguments"),
                ));
            };
            positional.push(*value);
        }
        let comptime_only = matches!(
            name,
            "struct_field_type"
                | "variant_field_type"
                | "tuple_append_type"
                | "field_value"
                | "comptime_field_entry"
                | "comptime_enum_variant_index"
                | "comptime_enum_variant_field_values"
        );
        if comptime_only && (self.runtime_value_context() || self.in_code_body) {
            return Err(error(
                expr.span,
                format!("{name} requires comptime invocation"),
            ));
        }
        let (ty, kind) = match name {
            "struct_field_type" => {
                let [s_arg, f_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "struct_field_type requires (Type, Symbol)",
                    ));
                };
                let subject = self.type_observation_operand(s_arg)?;
                let field = self.symbol_observation_operand(f_arg)?;
                let s_term = self.type_expr(s_arg)?;
                let f_term = self.symbol_type_term(f_arg)?;
                self.establish_type_requirement(
                    &n::TypePredicate::HasField {
                        subject: s_term,
                        field: f_term,
                    },
                    expr.span,
                )?;
                (
                    if is_never(&subject.ty) || is_never(&field.ty) {
                        p(P::Never)
                    } else {
                        p(P::Type)
                    },
                    K::StructFieldType {
                        subject: Box::new(subject),
                        field: Box::new(field),
                    },
                )
            }
            "variant_field_type" => {
                let [e_arg, v_arg, f_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "variant_field_type requires (Type, usize, usize)",
                    ));
                };
                let e_ty = self.type_expr(e_arg)?;
                let variant = self.expr(v_arg, Some(&p(P::Usize)))?;
                let field = self.expr(f_arg, Some(&p(P::Usize)))?;
                let v_term = self.usize_type_term(v_arg, &variant)?;
                let f_term = self.usize_type_term(f_arg, &field)?;
                let resolved = if let (
                    Term::Nominal {
                        declaration,
                        arguments,
                    },
                    Term::Integer(vi),
                    Term::Integer(fi),
                ) = (&e_ty, &v_term, &f_term)
                    && let Some(v_idx) = vi.to_u128().ok().and_then(|v| usize::try_from(v).ok())
                    && let Some(f_idx) = fi.to_u128().ok().and_then(|v| usize::try_from(v).ok())
                    && let Some(enum_def) = self.enums.get(declaration)
                    && let Some(raw_field) = enum_def
                        .variants
                        .get(v_idx)
                        .and_then(|v| v.fields.get(f_idx))
                {
                    let instantiated = self.instantiate_declaration_type(
                        raw_field,
                        &enum_def.type_parameters,
                        &enum_def.parameter_types,
                        arguments,
                        expr.span,
                    )?;
                    instantiated
                } else {
                    Term::variant_field_type(e_ty, v_term, f_term)
                };
                (p(P::Type), K::TypeValue(resolved))
            }
            "tuple_append_type" => {
                let [t_arg, e_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "tuple_append_type requires (Type, Type)"));
                };
                let t_ty = self.type_expr(t_arg)?;
                let e_ty = self.type_expr(e_arg)?;
                let result_ty = self.resolve_tuple_append_type(t_ty, e_ty);
                (p(P::Type), K::TypeValue(result_ty))
            }
            "field_ptr" => {
                let [ptr_arg, f_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "field_ptr requires (*S, Symbol)"));
                };
                let pointer = self.expr(ptr_arg, None)?;
                let field = self.symbol_observation_operand(f_arg)?;
                let field_term = self.symbol_type_term(f_arg)?;
                let Term::Pointer(pointee) = &pointer.ty else {
                    return Err(error(
                        ptr_arg.span,
                        "field_ptr requires a pointer to a struct or record",
                    ));
                };
                self.establish_type_requirement(
                    &n::TypePredicate::HasField {
                        subject: pointee.as_ref().clone(),
                        field: field_term.clone(),
                    },
                    expr.span,
                )?;
                let field_ty = Term::field_type(pointee.as_ref().clone(), field_term);
                (
                    Term::Pointer(Box::new(field_ty)),
                    K::FieldPtr {
                        pointer: Box::new(pointer),
                        field: Box::new(field),
                    },
                )
            }
            "field_value" => {
                let [val_arg, f_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "field_value requires (S, Symbol)"));
                };
                let value = self.expr(val_arg, None)?;
                let field = self.symbol_observation_operand(f_arg)?;
                let field_term = self.symbol_type_term(f_arg)?;
                self.establish_type_requirement(
                    &n::TypePredicate::HasField {
                        subject: value.ty.clone(),
                        field: field_term.clone(),
                    },
                    expr.span,
                )?;
                let field_ty = Term::field_type(value.ty.clone(), field_term);
                (
                    field_ty,
                    K::FieldValue {
                        value: Box::new(value),
                        field: Box::new(field),
                    },
                )
            }
            "comptime_field_entry" => {
                let [val_arg, f_arg, ty_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "comptime_field_entry requires (S, Symbol, Type)",
                    ));
                };
                let value = self.expr(val_arg, None)?;
                let field = self.expr(f_arg, Some(&p(P::Symbol)))?;
                let field_ty = self.type_observation_operand(ty_arg)?;
                (
                    if is_never(&value.ty) || is_never(&field.ty) || is_never(&field_ty.ty) {
                        p(P::Never)
                    } else {
                        p(P::Object)
                    },
                    K::ComptimeFieldEntry {
                        value: Box::new(value),
                        field: Box::new(field),
                        ty: Box::new(field_ty),
                    },
                )
            }
            "construct_struct" => {
                let [s_arg, fields_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "construct_struct requires (Type, Tuple)"));
                };
                let s_ty = self.type_expr(s_arg)?;
                let fields = self.expr(fields_arg, None)?;
                (
                    s_ty,
                    K::ConstructStruct {
                        fields: Box::new(fields),
                    },
                )
            }
            "tuple_append" => {
                let [tup_arg, elem_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "tuple_append requires (Tuple, E)"));
                };
                let tuple = self.expr(tup_arg, None)?;
                let element = self.expr(elem_arg, None)?;
                let result_ty =
                    self.resolve_tuple_append_type(tuple.ty.clone(), element.ty.clone());
                (
                    result_ty,
                    K::TupleAppend {
                        tuple: Box::new(tuple),
                        element: Box::new(element),
                    },
                )
            }
            "enum_variant_index" => {
                let [ptr_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "enum_variant_index requires (*E)"));
                };
                let pointer = self.expr(ptr_arg, None)?;
                let Term::Pointer(_) = &pointer.ty else {
                    return Err(error(
                        ptr_arg.span,
                        "enum_variant_index requires a pointer to an enum",
                    ));
                };
                (p(P::Usize), K::EnumVariantIndex(Box::new(pointer)))
            }
            "enum_variant_field_ptr" => {
                let [e_arg, ptr_arg, v_arg, f_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "enum_variant_field_ptr requires (Type, *E, usize, usize)",
                    ));
                };
                let e_ty = self.type_expr(e_arg)?;
                let pointer = self.expr(ptr_arg, Some(&Term::Pointer(Box::new(e_ty.clone()))))?;
                let variant = self.expr(v_arg, Some(&p(P::Usize)))?;
                let field = self.expr(f_arg, Some(&p(P::Usize)))?;
                let v_term = self.usize_type_term(v_arg, &variant)?;
                let f_term = self.usize_type_term(f_arg, &field)?;
                let field_ty = Term::variant_field_type(e_ty, v_term, f_term);
                (
                    Term::Optional(Box::new(Term::Pointer(Box::new(field_ty)))),
                    K::EnumVariantFieldPtr {
                        pointer: Box::new(pointer),
                        variant: Box::new(variant),
                        field: Box::new(field),
                    },
                )
            }
            "construct_enum" => {
                let [e_arg, v_arg, fields_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "construct_enum requires (Type, usize, Tuple)",
                    ));
                };
                let e_ty = self.type_expr(e_arg)?;
                let variant = self.expr(v_arg, Some(&p(P::Usize)))?;
                let fields = self.expr(fields_arg, None)?;
                (
                    e_ty,
                    K::ConstructEnum {
                        variant: Box::new(variant),
                        fields: Box::new(fields),
                    },
                )
            }
            "comptime_enum_variant_index" => {
                let [val_arg] = positional.as_slice() else {
                    return Err(error(expr.span, "comptime_enum_variant_index requires (E)"));
                };
                let value = self.expr(val_arg, None)?;
                (
                    if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        p(P::Integer)
                    },
                    K::ComptimeEnumVariantIndex(Box::new(value)),
                )
            }
            "comptime_enum_variant_field_values" => {
                let [val_arg] = positional.as_slice() else {
                    return Err(error(
                        expr.span,
                        "comptime_enum_variant_field_values requires (E)",
                    ));
                };
                let value = self.expr(val_arg, None)?;
                (
                    if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        p(P::Object)
                    },
                    K::ComptimeEnumVariantFieldValues(Box::new(value)),
                )
            }
            _ => unreachable!(),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind,
        })
    }
}
