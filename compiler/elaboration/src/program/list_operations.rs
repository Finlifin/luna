//! Privileged list primitives used by the ordinary meta package wrappers.
use super::*;

impl Checker<'_, '_> {
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
                && parameters.len() == 1 && parameters[0].2 == p(P::Type)
                && matches!(self.package.item(*owner).map(|item| &item.kind),
                    Some(ItemKind::Fn(sig, _)) if sig.modifiers.is_normal_form_head && sig.modifiers.is_pure)
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

    pub(super) fn build_enum_builtin(
        &mut self,
        expr: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        segmented_paths: bool,
    ) -> Result<n::Expr, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Err(error(
                expr.span,
                "enum construction requires comptime invocation",
            ));
        }
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
        let values = vec![
            self.expr(name, Some(&p(P::Symbol)))?,
            self.expr(groups, Some(&paths))?,
            self.expr(names, Some(&paths))?,
            self.expr(shapes, Some(&list(p(P::Symbol))))?,
            self.expr(field_names, Some(&list(list(p(P::Symbol)))))?,
            self.expr(field_types, Some(&list(list(p(P::Type)))))?,
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
