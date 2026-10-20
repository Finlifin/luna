//! Scoped effect-row equality inside one Boolean branch. No nested solver calls:
//! all label decisions use this branch's equivalence classes and disequalities,
//! so proof replay follows the same theory steps and cannot import foreign facts.
use super::*;
use nir::effects::Operation;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    Same,
    Different,
    Pending,
}

impl<'a> Search<'a> {
    pub(super) fn computation_congruent(
        &mut self,
        a: usize,
        b: usize,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<bool, UnknownReason> {
        if !matches!(self.terms.nodes[a], Term::Computation { .. })
            && !matches!(self.terms.nodes[b], Term::Computation { .. })
        {
            return Ok(false);
        }
        let Some((left, left_result)) = self.computation_view(a, classes)? else {
            return Ok(false);
        };
        let Some((right, right_result)) = self.computation_view(b, classes)? else {
            return Ok(false);
        };
        if left.len() != right.len() || left_result != right_result {
            return Ok(false);
        }
        self.steps = self
            .steps
            .checked_sub(right.len())
            .ok_or(UnknownReason::BudgetExhausted)?;
        let mut consumed = vec![false; right.len()];
        // Select the earliest equal unmatched occurrence. It can move to the
        // front only across proved-different labels. In particular, neither an
        // unknown label nor an earlier same-label occurrence may be skipped.
        for operation in &left {
            let mut found = false;
            for (index, candidate) in right.iter().enumerate() {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                if consumed[index] {
                    continue;
                }
                match self.operation_relation(operation, candidate, classes, assignment)? {
                    Relation::Same => {
                        consumed[index] = true;
                        found = true;
                        break;
                    }
                    Relation::Different => {}
                    Relation::Pending => return Ok(false),
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// A refutation rule, not failure of congruence. Unknown labels contribute
    /// to the maximum possible multiplicity and can never prove absence.
    pub(super) fn computation_distinct(
        &mut self,
        a: usize,
        b: usize,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<bool, UnknownReason> {
        // A nonempty effect prefix cannot be an ordinary data constructor.
        // Empty raw wrappers are deliberately excluded: canonical construction
        // erases those wrappers instead of making them a distinct Type.
        match (&self.terms.nodes[a], &self.terms.nodes[b]) {
            (Term::Computation { effects, .. }, other)
            | (other, Term::Computation { effects, .. })
                if !effects.is_empty()
                    && (constructor(other).is_some() || matches!(other, Term::Record(_))) =>
            {
                return Ok(true);
            }
            _ => {}
        }
        if !matches!(self.terms.nodes[a], Term::Computation { .. })
            && !matches!(self.terms.nodes[b], Term::Computation { .. })
        {
            return Ok(false);
        }
        let Some((left, lr)) = self.computation_view(a, classes)? else {
            return Ok(false);
        };
        let Some((right, rr)) = self.computation_view(b, classes)? else {
            return Ok(false);
        };
        // Prefix lengths refute equality only after residual results are known
        // ordinary constructors. Row views follow proved aliases, never an
        // opaque definition body or an assumed empty tail.
        let residuals = [lr, rr];
        let mut concrete = [false; 2];
        for (index, term) in self.terms.nodes.iter().enumerate() {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            if constructor(term).is_some() || matches!(term, Term::Record(_)) {
                for slot in 0..2 {
                    concrete[slot] |= classes.root(index) == residuals[slot];
                }
            }
        }
        if !concrete.into_iter().all(|known| known) {
            return Ok(false);
        }
        if left.len() != right.len() {
            return Ok(true);
        }
        if self.distinct_type_classes(lr, rr, classes, assignment, &mut BTreeSet::new(), 0)? {
            return Ok(true);
        }
        // Scoped-label commutations preserve multiplicities. A provable count
        // mismatch is contradictory even if other label relations are pending.
        for pivot in left.iter().chain(right.iter()) {
            let mut bounds = [(0usize, 0usize); 2];
            for (slot, row) in [&left, &right].into_iter().enumerate() {
                for candidate in row {
                    self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                    match self.operation_relation(pivot, candidate, classes, assignment)? {
                        Relation::Same => {
                            bounds[slot].0 += 1;
                            bounds[slot].1 += 1;
                        }
                        Relation::Pending => bounds[slot].1 += 1,
                        Relation::Different => {}
                    }
                }
            }
            if bounds[0].0 > bounds[1].1 || bounds[1].0 > bounds[0].1 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Flatten syntax and branch-proved result aliases. Returned operations
    /// borrow the immutable term table; no new type identity is manufactured.
    /// Empty self wrappers are harmless; nonempty cyclic tails remain pending.
    fn computation_view(
        &mut self,
        start: usize,
        classes: &Classes,
    ) -> Result<Option<(Vec<&'a Operation>, usize)>, UnknownReason> {
        if has_bound(&self.terms.nodes[start]) {
            return Ok(None);
        }
        let terms = self.terms;
        let mut current = start;
        let mut operations = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..self.max_depth.min(128) {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            let Term::Computation { effects, result } = &terms.nodes[current] else {
                return Ok(Some((operations, classes.root(current))));
            };
            self.steps = self
                .steps
                .checked_sub(effects.len())
                .ok_or(UnknownReason::BudgetExhausted)?;
            operations.extend(effects.iter());
            let result = terms.ids[result.as_ref()];
            let root = classes.root(result);
            if !seen.insert(root) {
                return Ok(None);
            }
            let mut next = None;
            let mut cyclic = false;
            // Start from the actual result when possible; all alternatives
            // belong to the same class and require explicit branch evidence.
            for index in std::iter::once(result).chain(0..terms.nodes.len()) {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                if classes.root(index) != root {
                    continue;
                }
                let Term::Computation { effects, result } = &terms.nodes[index] else {
                    continue;
                };
                if has_bound(&terms.nodes[index]) {
                    continue;
                }
                let target = classes.root(terms.ids[result.as_ref()]);
                if seen.contains(&target) {
                    cyclic |= !effects.is_empty();
                    continue;
                }
                next = Some(index);
                break;
            }
            match next {
                Some(index) => current = index,
                None if cyclic => return Ok(None),
                None => return Ok(Some((operations, root))),
            }
        }
        Err(UnknownReason::BudgetExhausted)
    }

    fn operation_relation(
        &mut self,
        a: &Operation,
        b: &Operation,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<Relation, UnknownReason> {
        if a.declaration != b.declaration || a.arguments.len() != b.arguments.len() {
            return Ok(Relation::Different);
        }
        let mut pending = false;
        for (a, b) in a.arguments.iter().zip(&b.arguments) {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            if a == b {
                continue;
            }
            if a.sort() != b.sort() {
                return Ok(Relation::Different);
            }
            if a.sort() != Sort::Type {
                // Value identity is not a PTC proposition. Only immutable
                // builtin scalar inequality is known here; other values wait.
                if matches!(
                    (a, b),
                    (Term::Bool(_), Term::Bool(_))
                        | (Term::Char(_), Term::Char(_))
                        | (Term::String(_), Term::String(_))
                        | (Term::Symbol(_), Term::Symbol(_))
                        | (Term::Integer(_), Term::Integer(_))
                        | (Term::BitVec(_), Term::BitVec(_))
                        | (Term::Float32(_), Term::Float32(_))
                        | (Term::Float64(_), Term::Float64(_))
                ) {
                    return Ok(Relation::Different);
                }
                pending = true;
                continue;
            }
            let ra = classes.root(self.terms.ids[a]);
            let rb = classes.root(self.terms.ids[b]);
            if ra == rb {
                continue;
            }
            if self.distinct_type_classes(ra, rb, classes, assignment, &mut BTreeSet::new(), 0)? {
                return Ok(Relation::Different);
            }
            pending = true;
        }
        Ok(if pending {
            Relation::Pending
        } else {
            Relation::Same
        })
    }

    fn distinct_type_classes(
        &mut self,
        a: usize,
        b: usize,
        classes: &Classes,
        assignment: &[Option<bool>],
        seen: &mut BTreeSet<(usize, usize)>,
        depth: usize,
    ) -> Result<bool, UnknownReason> {
        self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
        if depth >= self.max_depth.min(64) {
            return Err(UnknownReason::BudgetExhausted);
        }
        if a == b || !seen.insert((a.min(b), a.max(b))) {
            return Ok(false);
        }
        for (atom, value) in self.atoms.iter().zip(assignment) {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            if let (Atom::TypeEqual(Equality(left, right)), Some(false)) = (atom, value) {
                let left = classes.root(self.terms.ids[left]);
                let right = classes.root(self.terms.ids[right]);
                if (left == a && right == b) || (left == b && right == a) {
                    return Ok(true);
                }
            }
        }
        let mut left_head = None;
        let mut right_head = None;
        let mut left_index = None;
        let mut right_index = None;
        let mut left_record = false;
        let mut right_record = false;
        for (index, term) in self.terms.nodes.iter().enumerate() {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            let root = classes.root(index);
            if root == a {
                left_record |= matches!(term, Term::Record(_));
                if let Some(head) = constructor(term) {
                    left_head = Some(head);
                    left_index = Some(index);
                }
            } else if root == b {
                right_record |= matches!(term, Term::Record(_));
                if let Some(head) = constructor(term) {
                    right_head = Some(head);
                    right_index = Some(index);
                }
            }
        }
        // These are exactly the disjoint constructor families admitted by the
        // main equality theory. Opaque normal-form heads are never unfolded.
        match (&left_head, &right_head) {
            (Some(a), Some(b)) if a != b => return Ok(true),
            (Some(head), Some(_))
                if !matches!(head, Head::Nominal(..))
                    || matches!(head, Head::Nominal(id, _) if self.context.is_nominal_constructor(*id)) =>
            {
                let left = &self.terms.children[left_index.unwrap()];
                let right = &self.terms.children[right_index.unwrap()];
                for (&a, &b) in left.iter().zip(right) {
                    // Only Type children belong to this theory. Non-Type
                    // identity requires its separate value protocol.
                    if self.terms.nodes[a].sort() != Sort::Type
                        || self.terms.nodes[b].sort() != Sort::Type
                    {
                        continue;
                    }
                    if self.distinct_type_classes(
                        classes.root(a),
                        classes.root(b),
                        classes,
                        assignment,
                        seen,
                        depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
        Ok((left_record && right_head.is_some()) || (right_record && left_head.is_some()))
    }
}
