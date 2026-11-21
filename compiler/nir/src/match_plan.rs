//! Checked pattern-matching decision-DAG plans exported from elaboration to IIR and backends.

use crate::program::SourceRange;
use crate::{DeclarationId, Term};
use std::collections::BTreeMap;
use std::fmt;

/// Unique key identifying a compiled `match` site within an optional owner function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MatchPlanKey {
    pub function: Option<DeclarationId>,
    pub source: SourceRange,
}

impl fmt::Display for MatchPlanKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(match-key (function ")?;
        match self.function {
            Some(id) => write!(f, "{id}")?,
            None => f.write_str("none")?,
        }
        write!(f, ") {})", self.source)
    }
}

/// Reason a list pattern view is authorized for single-observation DAG lowering and length-interval proofs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ListObservationMode {
    Comptime,
    PureMethods,
    StableProtocol,
}

impl fmt::Display for ListObservationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Comptime => "comptime",
            Self::PureMethods => "pure-methods",
            Self::StableProtocol => "stable-protocol",
        })
    }
}

/// Selected list observation protocol and operations for a stable list-typed match place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListViewOps {
    pub protocol: DeclarationId,
    pub mode: ListObservationMode,
    pub by_value: bool,
    pub size_type: Term,
    pub element_type: Term,
    pub rest_type: Option<Term>,
    pub length_method: Option<DeclarationId>,
    pub element_method: Option<DeclarationId>,
    pub rest_method: Option<DeclarationId>,
}

impl fmt::Display for ListViewOps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(list-view-ops (protocol {}) (mode {}) (by-value {}) (size-type {}) (element-type {})",
            self.protocol, self.mode, self.by_value, self.size_type, self.element_type
        )?;
        if let Some(rest_type) = &self.rest_type {
            write!(f, " (rest-type {rest_type})")?;
        }
        if let Some(length_method) = self.length_method {
            write!(f, " (length-method {length_method})")?;
        }
        if let Some(element_method) = self.element_method {
            write!(f, " (element-method {element_method})")?;
        }
        if let Some(rest_method) = self.rest_method {
            write!(f, " (rest-method {rest_method})")?;
        }
        f.write_str(")")
    }
}

/// Compiled Maranget decision DAG and list-view resolution metadata for one `match`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPlan {
    pub source: SourceRange,
    pub scrutinee_type: Term,
    pub type_desc: pattern_automata::TypeDesc,
    pub dag: pattern_automata::DecisionDag,
    pub list_views: BTreeMap<pattern_automata::Place, ListViewOps>,
    pub unreachable_arms: Vec<usize>,
    pub is_exhaustive: bool,
    pub stats: pattern_automata::CompileStats,
    pub exact: bool,
}

impl MatchPlan {
    pub fn entry_count(&self) -> usize {
        1usize
            .saturating_add(self.dag.node_count())
            .saturating_add(self.list_views.len())
            .saturating_add(self.unreachable_arms.len())
    }
}

impl fmt::Display for MatchPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(match-plan {} (scrutinee-type {}) (type-desc {}) (exhaustive {}) (exact {}) (unreachable-arms",
            self.source, self.scrutinee_type, self.type_desc, self.is_exhaustive, self.exact
        )?;
        for arm in &self.unreachable_arms {
            write!(f, " {arm}")?;
        }
        f.write_str(") (list-views")?;
        for (place, ops) in &self.list_views {
            write!(f, " (view {place} {ops})")?;
        }
        write!(f, ") {} {})", self.stats, self.dag)
    }
}

/// Package-level table of compiled pattern decision DAGs indexed by `(function, source)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchPlans {
    pub plans: BTreeMap<MatchPlanKey, MatchPlan>,
}

impl MatchPlans {
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn entry_count(&self) -> usize {
        self.plans
            .values()
            .fold(0usize, |acc, plan| acc.saturating_add(plan.entry_count()))
    }

    pub fn insert(&mut self, key: MatchPlanKey, plan: MatchPlan) -> Option<MatchPlan> {
        self.plans.insert(key, plan)
    }

    pub fn get(&self, function: Option<DeclarationId>, source: SourceRange) -> Option<&MatchPlan> {
        if let Some(function) = function
            && let Some(plan) = self.plans.get(&MatchPlanKey {
                function: Some(function),
                source,
            })
        {
            return Some(plan);
        }
        self.plans.get(&MatchPlanKey {
            function: None,
            source,
        })
    }

    pub fn extend(&mut self, other: &Self) {
        for (key, plan) in &other.plans {
            self.plans.insert(*key, plan.clone());
        }
    }
}

impl fmt::Display for MatchPlans {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(match-plans")?;
        for (key, plan) in &self.plans {
            write!(f, "\n  (plan {key} {plan})")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Primitive;
    use pattern_automata::{CompileBudget, DagBuilder, LenInterval, Place, SwitchKind, TypeDesc};

    #[test]
    fn match_plans_display_uses_balanced_sexprs() {
        let mut builder = DagBuilder::new(CompileBudget::default());
        let empty_leaf = builder.leaf(0, Vec::new()).unwrap();
        let non_empty_leaf = builder.leaf(1, Vec::new()).unwrap();
        let root = builder
            .switch(
                Place::root(),
                SwitchKind::ListLen {
                    cases: vec![
                        (LenInterval::exact(0), empty_leaf),
                        (LenInterval::at_least(1), non_empty_leaf),
                    ],
                    default: None,
                },
            )
            .unwrap();
        let dag = builder.finalize(root);
        let source = SourceRange { start: 10, end: 42 };
        let fn_id = DeclarationId {
            package: 1,
            local: 7,
        };
        let mut list_views = BTreeMap::new();
        list_views.insert(
            Place::root(),
            ListViewOps {
                protocol: DeclarationId {
                    package: 0,
                    local: 3,
                },
                mode: ListObservationMode::StableProtocol,
                by_value: false,
                size_type: Term::Primitive(Primitive::Usize),
                element_type: Term::Primitive(Primitive::Bool),
                rest_type: Some(Term::Primitive(Primitive::Void)),
                length_method: Some(DeclarationId {
                    package: 1,
                    local: 9,
                }),
                element_method: None,
                rest_method: None,
            },
        );
        let mut plans = MatchPlans::default();
        plans.insert(
            MatchPlanKey {
                function: Some(fn_id),
                source,
            },
            MatchPlan {
                source,
                scrutinee_type: Term::Primitive(Primitive::Bool),
                type_desc: TypeDesc::List(Box::new(TypeDesc::Bool)),
                dag,
                list_views,
                unreachable_arms: vec![2],
                is_exhaustive: true,
                stats: pattern_automata::CompileStats {
                    steps_used: 3,
                    allocated_nodes: 3,
                    final_nodes: 3,
                    memo_hits: 0,
                },
                exact: true,
            },
        );

        let dumped = plans.to_string();
        assert!(dumped.starts_with("(match-plans"), "{dumped}");
        assert!(dumped.contains("stable-protocol"), "{dumped}");
        assert!(dumped.contains("(unreachable-arms 2)"), "{dumped}");
        let mut depth = 0isize;
        for ch in dumped.chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "unbalanced parens in: {dumped}");
        }
        assert_eq!(depth, 0, "unclosed parens in: {dumped}");
        assert!(plans.get(Some(fn_id), source).is_some());
        assert!(plans.get(None, source).is_none());
    }
}
