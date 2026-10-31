use nir::program::{ExpansionSource, SourceRange};

#[test]
fn generated_locations_resolve_originals_without_changing_identity() {
    let user = SourceRange { start: 11, end: 23 };
    let site = SourceRange { start: 30, end: 40 };
    let origins = [
        ExpansionSource {
            original: user,
            site,
        },
        ExpansionSource {
            original: SourceRange::generated(0),
            site,
        },
    ];
    let generated = SourceRange::generated(1);
    assert_eq!(generated.original(&origins), Some(user));
    assert_eq!(generated.expansion_index(), Some(1));
    assert_eq!(user.original(&[]), Some(user));
}

#[test]
fn invalid_provenance_does_not_become_a_file_location() {
    let generated = SourceRange::generated(0);
    let cycle = [ExpansionSource {
        original: generated,
        site: SourceRange { start: 1, end: 2 },
    }];
    assert_eq!(generated.original(&cycle), None);
    assert_eq!(generated.original(&[]), None);
    assert_eq!(SourceRange { start: 9, end: 2 }.original(&[]), None);
}
