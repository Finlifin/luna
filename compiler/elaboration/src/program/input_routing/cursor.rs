//! Incremental supply routing shared by whole calls and streamed input packets.
use super::{Entry, Failure, FailureKind, Input, post_lambda_target};
use std::{collections::HashMap, fmt, hash::Hash};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::program) struct Selection {
    pub parameter: usize,
    pub collected: bool,
}

/// This owns supply metadata, not source nodes or evaluated values. Unknown
/// packet arity permits only choices independent of the unseen packet fields.
#[derive(Debug)]
pub(in crate::program) struct Cursor<Name> {
    entries: Vec<Entry<Name>>,
    named: HashMap<Name, usize>,
    supplied: Vec<bool>,
    complete: Vec<bool>,
    positional: usize,
    remaining: Option<usize>,
    post_target: Option<usize>,
    anchored_suffix: Vec<usize>,
    variadic_suffix: Vec<usize>,
}

impl<Name: Clone + Eq + Hash> Cursor<Name> {
    pub fn new(
        entries: Vec<Entry<Name>>,
        inputs: &[Input<Name>],
        remaining: Option<usize>,
    ) -> Result<Self, Failure> {
        let count = entries.len();
        let post_target = post_lambda_target(&entries, inputs)?;
        let named: HashMap<_, _> = entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.name.clone().map(|name| (name, index)))
            .collect();
        let mut complete = vec![false; count];
        for (position, input) in inputs.iter().enumerate() {
            if let Input::Named(name) = input {
                let index = named
                    .get(name)
                    .copied()
                    .ok_or_else(|| FailureKind::UnknownName.at(position))?;
                if std::mem::replace(&mut complete[index], true) {
                    return Err(FailureKind::Duplicate.at(position));
                }
            }
        }
        let mut anchored_suffix = vec![0; count + 1];
        let mut variadic_suffix = vec![0; count + 1];
        for index in (0..count).rev() {
            let entry = &entries[index];
            variadic_suffix[index] =
                variadic_suffix[index + 1] + usize::from(entry.variadic && !complete[index]);
            anchored_suffix[index] = anchored_suffix[index + 1]
                + usize::from(!entry.skipped() && !entry.variadic && post_target != Some(index));
        }
        Ok(Self {
            entries,
            named,
            supplied: vec![false; count],
            complete,
            positional: 0,
            remaining,
            post_target,
            anchored_suffix,
            variadic_suffix,
        })
    }

    fn positional_entries(&self) -> impl Iterator<Item = usize> + '_ {
        (self.positional..self.entries.len())
            .filter(|index| !self.entries[*index].skipped() && self.post_target != Some(*index))
    }

    /// The next declared positional entry, before complete-input reservations.
    /// Packet consumers use this to distinguish a fixed packet from a spread
    /// governed by the collection protocol, including conflicting supplies.
    pub fn at_collector(&self) -> bool {
        self.positional_entries()
            .next()
            .is_some_and(|slot| self.entries[slot].variadic)
    }

    pub fn has_open_collector(&self) -> bool {
        self.variadic_suffix[0] != 0
    }

    pub fn fixed_packet_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.positional_entries()
            .take_while(|slot| !self.entries[*slot].variadic)
    }

    pub fn remaining_anchors(&self) -> usize {
        self.anchored_suffix[self.positional]
    }

    /// A tuple supplies one packet of fixed positional fields. Check its
    /// entire route before advancing the cursor or publishing any assignment.
    pub fn packet_slots(&mut self, width: usize) -> Result<Vec<usize>, FailureKind> {
        let mut slots = Vec::new();
        for slot in self.positional_entries().take(width) {
            if self.entries[slot].variadic {
                return Err(if self.complete[slot] {
                    FailureKind::AppendToComplete
                } else {
                    FailureKind::PacketTargetsCollection
                });
            }
            if self.supplied[slot] {
                return Err(FailureKind::Duplicate);
            }
            slots.push(slot);
        }
        if slots.len() != width {
            return Err(FailureKind::PacketTooWide);
        }
        if let Some(last) = slots.last() {
            self.positional = last + 1;
            self.remaining = self.remaining.map(|left| left.saturating_sub(width));
            for &slot in &slots {
                self.supplied[slot] = true;
            }
        }
        Ok(slots)
    }

    pub fn select(&mut self, input: &Input<Name>) -> Result<Selection, FailureKind> {
        // Compute a candidate without advancing. A rejected field or supply
        // cannot consume the next slot of a retained packet-routing task.
        let mut next = self.positional;
        let (parameter, collected) = match input {
            Input::Positional => {
                while let Some(entry) = self.entries.get(next) {
                    if self.post_target == Some(next) || entry.skipped() {
                        next += 1;
                        continue;
                    }
                    if entry.variadic {
                        if self.complete[next] {
                            next += 1;
                            continue;
                        }
                        match self.remaining {
                            Some(left) if left <= self.anchored_suffix[next + 1] => {
                                next += 1;
                                continue;
                            }
                            None if self.anchored_suffix[next + 1] != 0 => {
                                return Err(FailureKind::NeedPositionalArity);
                            }
                            _ => {}
                        }
                    }
                    break;
                }
                let Some(entry) = self.entries.get(next) else {
                    return Err(
                        if self
                            .entries
                            .iter()
                            .enumerate()
                            .any(|(index, entry)| entry.variadic && self.complete[index])
                        {
                            FailureKind::AppendToComplete
                        } else {
                            FailureKind::TooMany
                        },
                    );
                };
                if entry.variadic && self.variadic_suffix[next + 1] != 0 {
                    return Err(FailureKind::MultipleCollections);
                }
                (next, entry.variadic)
            }
            Input::Named(name) => (
                *self.named.get(name).ok_or(FailureKind::UnknownName)?,
                false,
            ),
            Input::PostLambda => (
                self.post_target.ok_or(FailureKind::MissingPostTarget)?,
                false,
            ),
            Input::Unsupported => return Err(FailureKind::Unsupported),
        };
        if self.supplied[parameter] && !collected {
            return Err(FailureKind::Duplicate);
        }
        if matches!(input, Input::Positional) {
            self.positional = next + usize::from(!collected);
            self.remaining = self.remaining.map(|left| left.saturating_sub(1));
        }
        self.supplied[parameter] = true;
        Ok(Selection {
            parameter,
            collected,
        })
    }

    pub fn missing_required(&self) -> bool {
        self.entries
            .iter()
            .zip(&self.supplied)
            .any(|(entry, supplied)| {
                !supplied
                    && ((entry.implicit && !entry.contextual)
                        || (!entry.optional && !entry.variadic && !entry.implicit))
            })
    }

    pub fn finish(&self) -> Result<(), FailureKind> {
        for (entry, supplied) in self.entries.iter().zip(&self.supplied) {
            if !supplied && entry.implicit && !entry.contextual {
                return Err(FailureKind::MissingImplicit);
            }
            if !supplied && !entry.skipped() && !entry.variadic {
                return Err(FailureKind::MissingAnchored);
            }
        }
        Ok(())
    }
}

impl fmt::Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-selection (parameter {}) (collected {}))",
            self.parameter, self.collected
        )
    }
}

impl<Name: fmt::Display> fmt::Display for Cursor<Name> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(input-cursor (positional {}) (remaining ",
            self.positional
        )?;
        match self.remaining {
            Some(count) => write!(f, "{count}")?,
            None => f.write_str("unknown")?,
        }
        f.write_str(") (post-target ")?;
        match self.post_target {
            Some(slot) => write!(f, "{slot}")?,
            None => f.write_str("none")?,
        }
        f.write_str(") (entries")?;
        for entry in &self.entries {
            write!(f, " {entry}")?;
        }
        for (name, bits) in [("complete", &self.complete), ("supplied", &self.supplied)] {
            write!(f, ") ({name}")?;
            for (slot, set) in bits.iter().enumerate() {
                if *set {
                    write!(f, " {slot}")?;
                }
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(name: &str) -> Entry<&str> {
        Entry {
            name: Some(name),
            variadic: true,
            post_target: false,
            ..Entry::anchored()
        }
    }

    #[test]
    fn fixed_packet_failure_preserves_the_tail_collection() {
        let mut cursor = Cursor::new(
            vec![Entry::anchored(), Entry::anchored(), collection("tail")],
            &[],
            None,
        )
        .unwrap();
        assert!(cursor.missing_required());
        let before = cursor.to_string();
        assert_eq!(
            cursor.packet_slots(3),
            Err(FailureKind::PacketTargetsCollection)
        );
        assert_eq!(cursor.to_string(), before);
        assert_eq!(cursor.packet_slots(2).unwrap(), [0, 1]);
        assert!(!cursor.missing_required());
        assert!(cursor.at_collector());
        let before = cursor.to_string();
        assert_eq!(
            cursor.packet_slots(1),
            Err(FailureKind::PacketTargetsCollection)
        );
        assert_eq!(cursor.to_string(), before);
        for _ in 0..3 {
            assert_eq!(
                cursor.select(&Input::Positional),
                Ok(Selection {
                    parameter: 2,
                    collected: true,
                })
            );
        }
        assert!(cursor.finish().is_ok());
    }

    #[test]
    fn unknown_packet_arity_cannot_guess_a_nonfinal_collection() {
        let entries = vec![collection("items"), Entry::anchored()];
        let mut cursor = Cursor::new(entries.clone(), &[], None).unwrap();
        let before = cursor.to_string();
        assert_eq!(
            cursor.select(&Input::Positional),
            Err(FailureKind::NeedPositionalArity)
        );
        assert_eq!(cursor.to_string(), before);

        // A complete supply makes the fixed packet's destination independent
        // of arity. Reserving that entry must not execute or consume its input.
        let mut complete = Cursor::new(entries, &[Input::Named("items")], None).unwrap();
        assert_eq!(complete.select(&Input::Positional).unwrap().parameter, 1);
        assert_eq!(
            complete.select(&Input::Named("items")),
            Ok(Selection {
                parameter: 0,
                collected: false,
            })
        );
        assert!(complete.finish().is_ok());
    }

    #[test]
    fn streamed_and_batch_selection_agree_without_losing_source_indices() {
        let entries = vec![
            Entry {
                name: Some("default"),
                optional: true,
                post_target: false,
                ..Entry::anchored()
            },
            Entry::anchored(),
            collection("tail"),
        ];
        let inputs = [
            Input::Positional,
            Input::Named("default"),
            Input::Positional,
            Input::Positional,
        ];
        let plan = super::super::route(&entries, &inputs).unwrap();
        let mut cursor = Cursor::new(entries, &inputs, None).unwrap();
        for (source, input) in inputs.iter().enumerate() {
            let selection = cursor.select(input).unwrap();
            let route = plan
                .routes
                .iter()
                .find(|route| route.sources.contains(&source))
                .unwrap();
            assert_eq!(selection.parameter, route.parameter);
            assert_eq!(selection.collected, route.collected);
        }
        assert!(cursor.finish().is_ok());
        assert!(!cursor.missing_required());
    }

    #[test]
    fn rejected_named_input_does_not_consume_a_pending_fixed_slot() {
        let entries = vec![
            Entry {
                name: Some("default"),
                optional: true,
                post_target: false,
                ..Entry::anchored()
            },
            Entry::anchored(),
        ];
        let mut cursor = Cursor::new(entries, &[Input::Named("default")], None).unwrap();
        let before = cursor.to_string();
        assert_eq!(
            cursor.select(&Input::Named("missing")),
            Err(FailureKind::UnknownName)
        );
        assert_eq!(cursor.to_string(), before);
        cursor.select(&Input::Named("default")).unwrap();
        let before = cursor.to_string();
        assert_eq!(
            cursor.select(&Input::Named("default")),
            Err(FailureKind::Duplicate)
        );
        assert_eq!(cursor.to_string(), before);
        assert_eq!(cursor.finish(), Err(FailureKind::MissingAnchored));
        assert_eq!(cursor.packet_slots(2), Err(FailureKind::PacketTooWide));
        assert_eq!(cursor.packet_slots(0).unwrap(), []);
        assert_eq!(cursor.packet_slots(1).unwrap(), [1]);
        assert!(cursor.finish().is_ok());
    }

    #[test]
    fn cursor_dump_distinguishes_reserved_and_delivered_inputs() {
        let mut cursor = Cursor::new(
            vec![collection("items\"\n")],
            &[Input::Named("items\"\n")],
            None,
        )
        .unwrap();
        let before = cursor.to_string();
        assert!(before.contains("(remaining unknown)"));
        assert!(before.contains("(name \"items\\\"\\n\")"));
        assert!(before.ends_with("(complete 0) (supplied))"));
        let selected = cursor.select(&Input::Named("items\"\n")).unwrap();
        assert_eq!(
            selected.to_string(),
            "(input-selection (parameter 0) (collected false))"
        );
        assert!(cursor.to_string().ends_with("(complete 0) (supplied 0))"));
    }
}
