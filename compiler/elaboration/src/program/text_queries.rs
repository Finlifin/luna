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
        let query = match name.as_str() {
            "str_byte_length" => n::TextQueryKind::ByteLength,
            "str_byte" => n::TextQueryKind::Byte,
            "str_slice" => n::TextQueryKind::Slice,
            "str_concat" => n::TextQueryKind::Concat,
            "str_from_char" => n::TextQueryKind::FromChar,
            "symbol_byte_length" => n::TextQueryKind::SymbolByteLength,
            "symbol_byte" => n::TextQueryKind::SymbolByte,
            _ => unreachable!(),
        };
        let expected_inputs: &[P] = match query {
            n::TextQueryKind::ByteLength => &[P::Str],
            n::TextQueryKind::Byte => &[P::Str, P::U64],
            n::TextQueryKind::SymbolByteLength => &[P::Symbol],
            n::TextQueryKind::SymbolByte => &[P::Symbol, P::U64],
            n::TextQueryKind::Slice => &[P::Str, P::U64, P::U64],
            n::TextQueryKind::Concat => &[P::Str, P::Str],
            n::TextQueryKind::FromChar => &[P::Char],
        };
        if args.len() != expected_inputs.len()
            || args.iter().any(|arg| !matches!(arg, Arg::Positional(_)))
        {
            return Err(error(
                expr.span,
                "text operation arguments do not match expected positional signature",
            ));
        }
        let mut values = Vec::new();
        for (arg, input) in args.iter().zip(expected_inputs) {
            let Arg::Positional(arg) = arg else {
                unreachable!()
            };
            values.push(self.expr(arg, Some(&p(*input)))?);
        }
        let ty = if values.iter().any(|value| is_never(&value.ty)) {
            p(P::Never)
        } else {
            match query {
                n::TextQueryKind::ByteLength | n::TextQueryKind::SymbolByteLength => p(P::U64),
                n::TextQueryKind::Byte | n::TextQueryKind::SymbolByte => {
                    Term::Optional(Box::new(p(P::U8)))
                }
                n::TextQueryKind::Slice => Term::Optional(Box::new(p(P::Str))),
                n::TextQueryKind::Concat | n::TextQueryKind::FromChar => p(P::Str),
            }
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::TextQuery {
                query,
                subject: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
                    kind: K::Tuple(values),
                }),
            },
        })
    }
}
