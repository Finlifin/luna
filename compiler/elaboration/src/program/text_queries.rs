use super::*;

impl Checker<'_, '_> {
    pub(super) fn text_builtin(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::Builtin(name) = &callee.kind else {
            unreachable!()
        };
        let length = name.as_str() == "str_byte_length";
        if args.len() != if length { 1 } else { 2 }
            || args.iter().any(|arg| !matches!(arg, Arg::Positional(_)))
        {
            return Err(error(
                expr.span,
                "text primitive requires positional str and optional u64 index",
            ));
        }
        let mut values = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let Arg::Positional(arg) = arg else {
                unreachable!()
            };
            values.push(self.expr(arg, Some(&p(if index == 0 { P::Str } else { P::U64 })))?);
        }
        let ty = if values.iter().any(|value| is_never(&value.ty)) {
            p(P::Never)
        } else if length {
            p(P::U64)
        } else {
            Term::Optional(Box::new(p(P::U8)))
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::TextQuery {
                query: if length {
                    n::TextQueryKind::ByteLength
                } else {
                    n::TextQueryKind::Byte
                },
                subject: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
                    kind: K::Tuple(values),
                }),
            },
        })
    }
}
