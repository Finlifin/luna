//! Inherent member domain. Names restrict candidates before structural ranking.
//! It shares subject matching with traits, never invents a trait identity and
//! never grants an open trait head permission to select an inherent method.
use super::*;
use crate::ptc::Context;
use nir::program::TypePredicate;

#[derive(Clone, Debug)]
pub struct InherentHead {
    /// Ordered declaration domains; empty denotes all Type parameters.
    pub parameter_types: Vec<Term>,
    pub declaration: DeclarationId,
    pub subject: Term,
    pub type_parameters: Vec<DeclarationId>,
    pub requirements: Vec<TypePredicate>,
    pub members: BTreeSet<String>,
}
impl InherentHead {
    fn matching_head(&self) -> ImplementationHead {
        ImplementationHead {
            parameter_types: self.parameter_types.clone(),
            declaration: self.declaration,
            subject: self.subject.clone(),
            type_parameters: self.type_parameters.clone(),
            requirements: self.requirements.clone(),
            interface: None,
            interface_arguments: vec![],
        }
    }
}
#[derive(Default)]
pub struct InherentIndex {
    index: Index,
    heads: BTreeMap<DeclarationId, InherentHead>,
}
impl InherentIndex {
    pub fn for_builtin_packages(
        target: Option<nir::target::TargetData>,
        packages: impl IntoIterator<Item = u64>,
    ) -> Self {
        Self {
            index: Index::for_builtin_packages(target, packages),
            heads: BTreeMap::new(),
        }
    }

    pub fn check_ownership(&self, subject: &Term, package: u64) -> Result<Ownership, Error> {
        if self.index.builtin_packages.contains(&package) && ownership::builtin_subject(subject) {
            Ok(Ownership::BuiltinPackage(package))
        } else {
            inherent_ownership(subject, package)
        }
    }

    pub fn for_target(target: Option<nir::target::TargetData>) -> Self {
        Self {
            index: Index::for_target(target),
            heads: BTreeMap::new(),
        }
    }

    pub fn set_nominal_layouts(
        &mut self,
        structs: Vec<nir::program::Struct>,
        enums: Vec<nir::program::Enum>,
        newtypes: Vec<nir::newtypes::Newtype>,
    ) {
        self.index.set_nominal_layouts(structs, enums, newtypes);
    }

    pub fn set_normal_form_metadata(&mut self, heads: Vec<nir::normal_forms::Head>) {
        self.index.set_normal_form_metadata(heads);
    }

    /// Avoid copying complete layouts on repeated method lookup in an unchanged
    /// declaration world. Metadata order is canonical before comparison.
    pub fn sync_nominal_layouts<'a>(
        &mut self,
        structs: impl IntoIterator<Item = &'a nir::program::Struct>,
        enums: impl IntoIterator<Item = &'a nir::program::Enum>,
        newtypes: impl IntoIterator<Item = &'a nir::newtypes::Newtype>,
    ) {
        self.index.sync_nominal_layouts(structs, enums, newtypes);
    }

    /// Register immutable caller input domains without granting a member or trait.
    pub fn register_parameter_domain(
        &mut self,
        declaration: DeclarationId,
        domain: Term,
    ) -> Result<(), String> {
        self.index.register_parameter_domain(declaration, domain)
    }

    pub fn register_runtime_domain(
        &mut self,
        runtime: nir::RuntimeId,
        domain: Term,
    ) -> Result<(), String> {
        self.index.register_runtime_domain(runtime, domain)
    }

    pub fn insert(&mut self, head: InherentHead, budget: Budget) -> Result<(), Error> {
        self.check_ownership(&head.subject, head.declaration.package)?;
        if head.members.len() > budget.steps {
            return Err(Error::BudgetExhausted);
        }
        self.index.insert(
            head.matching_head(),
            Budget {
                steps: budget.steps - head.members.len(),
                ..budget
            },
        )?;
        self.heads.insert(head.declaration, head);
        Ok(())
    }
    pub fn resolution_is_current(&self, resolution: &TypeResolution) -> bool {
        self.index.type_resolution_is_current(resolution)
    }
    pub fn resolve_types(
        &self,
        name: &str,
        subject: &Term,
        context: &Context,
        budget: Budget,
    ) -> Result<TypeResolution, Error> {
        self.resolve_types_with(name, subject, context, budget, |_, _| {
            Applicability::Applicable
        })
    }
    pub(crate) fn resolve_types_with(
        &self,
        name: &str,
        subject: &Term,
        context: &Context,
        budget: Budget,
        mut qualify: impl FnMut(&TypeEvidence, Budget) -> Applicability,
    ) -> Result<TypeResolution, Error> {
        self.resolve_matched_with(name, subject, context, budget, |_, matched, allowance| {
            let assessment = match matched {
                Ok(matched) => matched.discharge_with(|goal, budget| context.prove(goal, budget)),
                Err(assessment) => assessment,
            };
            let state = match &assessment {
                TypeAssessment::Applicable(evidence) => qualify(evidence, allowance),
                _ => assessment.applicability(),
            };
            (assessment, state)
        })
    }
    pub(crate) fn resolve_matched_with(
        &self,
        name: &str,
        subject: &Term,
        context: &Context,
        budget: Budget,
        mut assess: impl FnMut(
            DeclarationId,
            Result<HeadMatch, TypeAssessment>,
            Budget,
        ) -> (TypeAssessment, Applicability),
    ) -> Result<TypeResolution, Error> {
        self.index.resolve_in_domain_with_search_budget(
            None,
            &[],
            subject,
            context,
            &mut { budget },
            |candidates| {
                candidates
                    .declarations
                    .retain(|id| self.heads[id].members.contains(name));
                // A member name fixes this domain without inventing a trait.
                candidates.fixed_interface = true;
            },
            |head, allowance| {
                let matched = head.match_subject_types_with_domains(
                    subject,
                    &[],
                    context,
                    allowance,
                    &self.index.parameter_domains,
                    &self.index.runtime_domains,
                    Some(&self.index.nominal_layouts),
                );
                assess(head.declaration, matched, allowance)
            },
        )
    }
    pub fn verify_evidence(
        &self,
        name: &str,
        evidence: &TypeEvidence,
        subject: &Term,
        context: &Context,
        budget: Budget,
    ) -> Result<(), EvidenceError> {
        self.verify_evidence_with(name, evidence, subject, context, budget, |proof, budget| {
            context.verify_proof(proof, budget)
        })
    }
    pub(crate) fn verify_evidence_with(
        &self,
        name: &str,
        evidence: &TypeEvidence,
        subject: &Term,
        context: &Context,
        budget: Budget,
        verify_requirement: impl FnMut(&crate::ptc::Proof, Budget) -> Result<(), crate::ptc::ProofError>,
    ) -> Result<(), EvidenceError> {
        let head = self
            .heads
            .get(&evidence.implementation)
            .ok_or(crate::ptc::ProofError::Invalid)?;
        if !head.members.contains(name) {
            return Err(crate::ptc::ProofError::Invalid.into());
        }
        self.index.entries[&head.declaration]
            .head
            .verify_subject_evidence_with_domains(
                evidence,
                subject,
                &[],
                context,
                budget,
                &self.index.parameter_domains,
                &self.index.runtime_domains,
                Some(&self.index.nominal_layouts),
                verify_requirement,
            )
    }
    /// Declaration-world check, independent of local caller assumptions. Only
    /// heads with a shared member name, equal precision, and non-distinct outer
    /// type heads require solver overlap queries. Unknown never means disjoint.
    pub fn check_coherence(&self, context: &Context, budget: Budget) -> Result<(), String> {
        let mut by_member: BTreeMap<&str, Vec<DeclarationId>> = BTreeMap::new();
        let mut member_entries = 0usize;
        for head in self.heads.values() {
            member_entries = member_entries
                .checked_add(head.members.len())
                .ok_or("inherent coherence budget exhausted")?;
            for name in &head.members {
                by_member
                    .entry(name.as_str())
                    .or_default()
                    .push(head.declaration);
            }
        }
        let mut pairs = 0usize;
        for decls in by_member.values() {
            let group_pairs = decls
                .len()
                .checked_mul(decls.len().saturating_sub(1))
                .and_then(|n| n.checked_div(2))
                .ok_or("inherent coherence budget exhausted")?;
            pairs = pairs
                .checked_add(group_pairs)
                .ok_or("inherent coherence budget exhausted")?;
        }
        let scan_cost = self
            .heads
            .len()
            .checked_add(member_entries)
            .and_then(|n| n.checked_add(pairs))
            .ok_or("inherent coherence budget exhausted")?;
        let remaining = budget
            .steps
            .checked_sub(scan_cost)
            .ok_or("inherent coherence budget exhausted")?;
        let mut seen_pairs = BTreeSet::new();
        let mut solver_pairs = Vec::new();
        for (name, decls) in by_member {
            for (i, &left_decl) in decls.iter().enumerate() {
                for &right_decl in &decls[i + 1..] {
                    if !seen_pairs.insert((left_decl, right_decl)) {
                        continue;
                    }
                    let a = &self.index.entries[&left_decl];
                    let b = &self.index.entries[&right_decl];
                    if a.precision != b.precision {
                        continue;
                    }
                    if crate::ptc::distinct_outer_type_heads(&a.head.subject, &b.head.subject) {
                        continue;
                    }
                    solver_pairs.push((name, left_decl, right_decl));
                }
            }
        }
        if solver_pairs.is_empty() {
            return Ok(());
        }
        let allowance = Budget {
            steps: remaining / solver_pairs.len(),
            ..budget
        };
        if allowance.steps == 0 {
            return Err("inherent coherence budget exhausted".into());
        }
        for (name, left_decl, right_decl) in solver_pairs {
            let a = &self.index.entries[&left_decl].head;
            let b = &self.index.entries[&right_decl].head;
            let assessment = a.assess_subject_overlap(b, context, allowance);
            let valid = match &assessment {
                PairAssessment::OrderedPrecision { .. } => true,
                PairAssessment::Disjoint(proof) => {
                    a.verify_subject_disjointness(b, proof, context, allowance)
                }
                _ => false,
            };
            if !valid {
                return Err(format!(
                    "inherent implementation overlap for {name:?} between {left_decl} and {right_decl}: {assessment}"
                ));
            }
        }
        Ok(())
    }
}
impl fmt::Display for InherentHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(inherent-head {} (subject {}) (parameters",
            self.declaration, self.subject
        )?;
        for p in &self.type_parameters {
            write!(f, " {p}")?;
        }
        write!(f, ") (parameter-types")?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
        }
        write!(f, ") (requires")?;
        for p in &self.requirements {
            write!(f, " {p}")?;
        }
        write!(f, ") (members")?;
        for name in &self.members {
            write!(f, " {name:?}")?;
        }
        write!(f, "))")
    }
}
impl fmt::Display for InherentIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(inherent-index (revision {}) (heads",
            self.index.revision
        )?;
        for head in self.heads.values() {
            write!(f, " {head}")?;
        }
        f.write_str(")")?;
        self.index.fmt_builtin_packages(f)?;
        self.index.fmt_nominal_layouts(f)?;
        for (id, domain) in &self.index.parameter_domains {
            write!(f, " (parameter-domain {id} {domain})")?;
        }
        f.write_str(")")
    }
}
