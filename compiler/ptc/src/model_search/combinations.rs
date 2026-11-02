//! Enumerate only the new part of a Cartesian product after domains grow.
//! Partition by the first coordinate using a newly added value, so the regions
//! are disjoint and exhausted earlier combinations are not scanned again.

pub(super) fn first(sizes: &[usize], previous: &[usize]) -> Option<Vec<usize>> {
    if sizes.len() != previous.len()
        || sizes
            .iter()
            .zip(previous)
            .any(|(size, old)| size < old || *size == 0)
    {
        return None;
    }
    if sizes.is_empty() {
        return Some(Vec::new());
    }
    let pivot = next_pivot(0, sizes, previous)?;
    let mut positions = vec![0; sizes.len()];
    positions[pivot] = previous[pivot];
    Some(positions)
}

pub(super) fn advance(positions: &mut [usize], sizes: &[usize], previous: &[usize]) -> bool {
    let Some(pivot) = positions
        .iter()
        .zip(previous)
        .position(|(position, old)| position >= old)
    else {
        return false;
    };
    for index in (0..sizes.len()).rev() {
        let limit = if index < pivot {
            previous[index]
        } else {
            sizes[index]
        };
        if positions[index] + 1 < limit {
            positions[index] += 1;
            for later in index + 1..sizes.len() {
                positions[later] = if later == pivot { previous[later] } else { 0 };
            }
            return true;
        }
    }
    let Some(next) = next_pivot(pivot + 1, sizes, previous) else {
        return false;
    };
    positions.fill(0);
    positions[next] = previous[next];
    true
}

fn next_pivot(start: usize, sizes: &[usize], previous: &[usize]) -> Option<usize> {
    if previous[..start].contains(&0) {
        return None;
    }
    for pivot in start..sizes.len() {
        if sizes[pivot] > previous[pivot] {
            return Some(pivot);
        }
        if previous[pivot] == 0 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn growing_domains_visit_exactly_the_new_combinations_once() {
        for sizes in [[1, 1, 1], [2, 3, 2], [3, 2, 4]] {
            for a in 0..=sizes[0] {
                for b in 0..=sizes[1] {
                    for c in 0..=sizes[2] {
                        let previous = [a, b, c];
                        let expected: BTreeSet<_> = (0..sizes[0])
                            .flat_map(|x| {
                                (0..sizes[1])
                                    .flat_map(move |y| (0..sizes[2]).map(move |z| vec![x, y, z]))
                            })
                            .filter(|positions| {
                                positions
                                    .iter()
                                    .zip(previous)
                                    .any(|(value, old)| *value >= old)
                            })
                            .collect();
                        let mut actual = BTreeSet::new();
                        if let Some(mut positions) = first(&sizes, &previous) {
                            loop {
                                assert!(actual.insert(positions.clone()), "duplicate combination");
                                if !advance(&mut positions, &sizes, &previous) {
                                    break;
                                }
                            }
                        }
                        assert_eq!(actual, expected, "sizes={sizes:?}, previous={previous:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn empty_and_unchanged_products_terminate() {
        assert_eq!(first(&[], &[]), Some(vec![]));
        assert!(!advance(&mut [], &[], &[]));
        assert_eq!(first(&[0], &[0]), None);
        assert_eq!(first(&[3, 4], &[3, 4]), None);
        assert_eq!(first(&[2], &[3]), None);
    }
}
