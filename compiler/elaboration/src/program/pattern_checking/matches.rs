//! Shared arm pattern/guard checking and match coverage requirements.
use super::*;

impl crate::automata::AutomataEnv for Checker<'_, '_> {
    fn structural(&self, ty: &Term) -> Option<Term> {
        self.structural_type(ty, rustc_span::DUMMY_SP).ok()
    }

    fn integer_layout(&self, primitive: nir::Primitive) -> Option<(bool, u32)> {
        self.integer_layout(primitive)
    }

    fn list_element_type<'t>(&self, ty: &'t Term) -> Option<&'t Term> {
        self.list_element_type(ty)
    }

    fn enum_def(&self, id: DeclarationId) -> Option<&n::Enum> {
        self.enums.get(&id)
    }

    fn struct_def(&self, id: DeclarationId) -> Option<&n::Struct> {
        self.structs.get(&id)
    }

    fn instantiate_enum_variant_fields(
        &self,
        def: &n::Enum,
        args: &[Term],
    ) -> Option<Vec<Vec<Term>>> {
        def.instantiate_variant_fields(args, self.context.scope(), &mut 100_000)
            .ok()
    }

    fn instantiate_struct_fields(
        &self,
        def: &n::Struct,
        args: &[Term],
    ) -> Option<Vec<(String, Term)>> {
        let field_tys = def
            .instantiate_stored_fields(args, self.context.scope(), &mut 100_000)
            .ok()?;
        Some(
            def.fields
                .iter()
                .map(|(name, _)| name.clone())
                .zip(field_tys)
                .collect(),
        )
    }

    fn list_pattern_info(&self, binding: n::LocalId) -> Option<&crate::automata::ListPatternInfo> {
        self.list_pattern_infos.get(&binding)
    }

    fn range_pattern_info(
        &self,
        binding: n::LocalId,
    ) -> Option<&crate::automata::RangePatternInfo> {
        self.range_pattern_infos.get(&binding)
    }

    fn bitvec_pattern_info(
        &self,
        binding: n::LocalId,
    ) -> Option<&crate::automata::BitVecPatternInfo> {
        self.bitvec_pattern_infos.get(&binding)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn require_match_coverage(
        &mut self,
        patterns: &[&n::Pattern],
        ty: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let site = source(span);
        let outcome = crate::automata::compile_match_plan(self, patterns, ty, site);
        if let Some(ref outcome) = outcome {
            self.match_plans.insert(
                n::MatchPlanKey {
                    function: self.active_body_declaration,
                    source: site,
                },
                outcome.plan.clone(),
            );
            if outcome.plan.is_exhaustive {
                return Ok(());
            }
        }
        match self.patterns_exhaustive(patterns, ty) {
            crate::coverage::Coverage::Exhaustive => Ok(()),
            crate::coverage::Coverage::Unknown => Err(error(
                span,
                "match exhaustiveness could not be proved within supported analysis and budget; add an unguarded fallback pattern",
            )),
            _ => {
                if let Some(ref outcome) = outcome
                    && outcome.plan.exact
                    && !outcome.missing_witnesses.is_empty()
                {
                    let witnesses = crate::automata::format_missing_witnesses(
                        self,
                        &outcome.missing_witnesses,
                        ty,
                        &outcome.plan.list_views,
                    );
                    if !witnesses.is_empty() {
                        return Err(error(
                            span,
                            format!(
                                "non-exhaustive match; missing pattern(s): {witnesses}; add an unguarded fallback pattern"
                            ),
                        ));
                    }
                }
                Err(error(
                    span,
                    "non-exhaustive match; add an unguarded fallback pattern",
                ))
            }
        }
    }

    pub(in crate::program) fn match_arm_pattern(
        &mut self,
        arm: &hir::pattern::PatternArm<'_>,
        value: &n::Expr,
    ) -> Result<n::Pattern, Error> {
        let observed = self.known_pattern_type_value(value, arm.pat.span)?;
        let domain = arguments::inference_type(value).unwrap_or(&value.ty);
        let pattern = self.pattern_observed(&arm.pat, domain, observed.as_ref())?;
        let guard = arm
            .guard
            .map(|guard| {
                self.under_matched_pattern_type_facts(&pattern, value, guard.span, |checker| {
                    checker.expr(guard, Some(&p(P::Bool)))
                })
            })
            .transpose()?;
        // Final guard runs after candidate selection, including or-patterns.
        Ok(match guard {
            Some(condition) => n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            },
            None => pattern,
        })
    }
}
