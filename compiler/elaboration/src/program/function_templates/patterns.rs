//! Pattern binders and successful branch outputs share the expression identity map.
use super::*;
use hir::pattern::{BitSegment, FieldPat, Pattern, PatternArm, PatternKind as P, TypePatternArg};

impl<'h> Rebind<'_, 'h> {
    pub(super) fn pattern(
        &mut self,
        pattern: &Pattern<'h>,
        depth: usize,
    ) -> Result<&'h Pattern<'h>, Error> {
        self.charge(pattern.span, depth)?;
        let depth = depth + 1;
        let kind = match &pattern.kind {
            P::Wild | P::OptionNull | P::Async | P::Invalid => pattern.kind.clone(),
            P::Binding(mode, name, inner) => P::Binding(
                *mode,
                *name,
                inner.map(|p| self.pattern(p, depth)).transpose()?,
            ),
            P::Projection(p, name) => P::Projection(self.pattern(p, depth)?, *name),
            P::Const(e) => P::Const(self.expr(e, depth)?),
            P::Comptime(e) => P::Comptime(self.expr(e, depth)?),
            P::Then(a, e, b) => P::Then(
                self.pattern(a, depth)?,
                self.expr(e, depth)?,
                self.pattern(b, depth)?,
            ),
            P::Guard(p, e) => P::Guard(self.pattern(p, depth)?, self.expr(e, depth)?),
            P::Tuple(ps) => P::Tuple(self.patterns(ps, depth)?),
            P::Or(ps) => P::Or(self.patterns(ps, depth)?),
            P::List(ps, rest) => P::List(
                self.patterns(ps, depth)?,
                rest.map(|p| self.pattern(p, depth)).transpose()?,
            ),
            P::AppTuple(head, ps) => {
                P::AppTuple(self.pattern(head, depth)?, self.patterns(ps, depth)?)
            }
            P::Struct(head, fields, rest) => P::Struct(
                self.pattern(head, depth)?,
                self.pattern_fields(fields, depth)?,
                *rest,
            ),
            P::AppStruct(head, fields) => P::AppStruct(
                self.pattern(head, depth)?,
                self.pattern_fields(fields, depth)?,
            ),
            P::OptionSome(p) => P::OptionSome(self.pattern(p, depth)?),
            P::ErrorOk(p) => P::ErrorOk(self.pattern(p, depth)?),
            P::ErrorErr(p) => P::ErrorErr(self.pattern(p, depth)?),
            P::Not(p) => P::Not(self.pattern(p, depth)?),
            P::Ref(p) => P::Ref(self.pattern(p, depth)?),
            P::Range(a, b, bound) => P::Range(
                a.map(|e| self.expr(e, depth)).transpose()?,
                b.map(|e| self.expr(e, depth)).transpose()?,
                bound.clone(),
            ),
            P::NormalForm(head, args) => {
                let head = self.expr(head, depth)?;
                let args = args
                    .iter()
                    .map(|arg| {
                        Ok(TypePatternArg {
                            name: arg.name,
                            pattern: self.pattern(&arg.pattern, depth)?.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                P::NormalForm(head, self.arena.alloc_type_pattern_arg_slice(args))
            }
            P::CodeSplice(e, outputs) => {
                let outputs = outputs
                    .iter()
                    .map(|output| {
                        let mut output = output.clone();
                        output.hir_id = self.id(output.hir_id);
                        output
                    })
                    .collect();
                P::CodeSplice(self.expr(e, depth)?, outputs)
            }
            P::BitVec { radix, segments } => P::BitVec {
                radix: *radix,
                segments: segments
                    .iter()
                    .map(|segment| {
                        Ok(match segment {
                            BitSegment::Literal { .. } => segment.clone(),
                            BitSegment::Typed { pattern, ty } => BitSegment::Typed {
                                pattern: self.pattern(pattern, depth)?,
                                ty: self.expr(ty, depth)?,
                            },
                            BitSegment::Computed(e) => BitSegment::Computed(self.expr(e, depth)?),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
            },
        };
        let hir_id = self.id(pattern.hir_id);
        Ok(self.arena.alloc_pattern(Pattern {
            hir_id,
            kind,
            span: pattern.span,
        }))
    }
    fn patterns(&mut self, ps: &[Pattern<'h>], depth: usize) -> Result<&'h [Pattern<'h>], Error> {
        let ps = ps
            .iter()
            .map(|p| self.pattern(p, depth).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.arena.alloc_pattern_slice(ps))
    }
    fn pattern_fields(
        &mut self,
        fields: &[FieldPat<'h>],
        depth: usize,
    ) -> Result<&'h [FieldPat<'h>], Error> {
        let fields = fields
            .iter()
            .map(|field| {
                Ok(FieldPat {
                    ident: field.ident,
                    pat: self.pattern(&field.pat, depth)?.clone(),
                    span: field.span,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(self.arena.alloc_field_pat_slice(fields))
    }
    pub(super) fn arm(
        &mut self,
        arm: &PatternArm<'h>,
        depth: usize,
    ) -> Result<PatternArm<'h>, Error> {
        Ok(PatternArm {
            hir_id: self.id(arm.hir_id),
            pat: self.pattern(&arm.pat, depth)?.clone(),
            guard: arm.guard.map(|e| self.expr(e, depth)).transpose()?,
            body: self.expr(arm.body, depth)?,
            span: arm.span,
        })
    }
    pub(super) fn arms(
        &mut self,
        arms: &[PatternArm<'h>],
        depth: usize,
    ) -> Result<&'h [PatternArm<'h>], Error> {
        let arms = arms
            .iter()
            .map(|a| self.arm(a, depth))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.arena.alloc_arm_slice(arms))
    }
}
