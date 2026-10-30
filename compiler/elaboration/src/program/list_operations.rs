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
            .package
            .owners()
            .find_map(|(owner, _)| {
                matches_declaration(self.package, &self.package_identities, owner, head)
                    .then_some(owner)
            })
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
        let admitted = self.normal_parameters.iter().any(|(owner, parameters)| {
            declaration(self.package, &self.package_identities, *owner) == head
                && parameters.len() == 1
                && parameters[0].2 == p(P::Type)
                && self.is_registered_normal_form(*owner)
        });
        if admitted {
            self.context.admit_list_inference_head(head);
        }
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
        let names = name.as_str() == "struct_field_names";
        let head = self
            .protocols
            .meta_list
            .ok_or_else(|| error(expr.span, "struct reflection requires a bound meta.List<T>"))?;
        self.ensure_meta_list_metadata(head, expr.span)?;
        let subject = self.expr(subject, Some(&p(P::Type)))?;
        let ty = if is_never(&subject.ty) {
            p(P::Never)
        } else {
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![p(if names { P::Symbol } else { P::Type })],
            }
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: if names {
                K::StructFieldNames(Box::new(subject))
            } else {
                K::StructFieldTypes(Box::new(subject))
            },
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
        use n::EnumStructureQuery as Q;
        let query = match name.as_str() {
            "enum_group_paths" => Q::GroupPaths,
            "enum_variant_paths" => Q::VariantPaths,
            "enum_variant_shapes" => Q::Shapes,
            "enum_variant_field_names" => Q::FieldNames,
            "enum_variant_field_types" => Q::FieldTypes,
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
        } else if query == Q::Shapes {
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
}
