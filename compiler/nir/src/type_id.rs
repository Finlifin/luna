//! Deterministic 128-bit Merkle ABI `type_id` fingerprint computation.
//!
//! Every concrete ABI-bearing type computes a 128-bit digest from:
//! 1. Canonical full path + effective version (`__flurry_stable_since` inherited
//!    from normal-form definition attributes, or the owning package SemVer), plus
//!    canonicalized type/comptime-value/globally-lifted runtime arguments;
//! 2. Target ABI layout (`LayoutWithNiche`, representation mode, field/bit offsets,
//!    and enum discriminant/encoding specifications);
//! 3. All transitive child symbol dependencies encoded recursively with a fixed
//!    preorder recursion stack so cyclic pointer graphs terminate deterministically.
//!
//! Raw `0u128` digests are mapped deterministically to `1u128` so `0` remains
//! exclusively reserved for `!E T` `Ok` paths.
use crate::layout::{
    LayoutError, LayoutWithNiche, LayoutWorld, Niche, NicheScalar, PackageLayoutWorld, align_up,
    bit_packed_struct_layout, compute_layout_and_niche, sum_tag_bits, unit_enum_tag_bits,
};
use crate::program::{
    CallingConvention, CanonicalDeclarationPath, DefinitionAttributes, EffectiveAttributeError,
    EffectiveAttributes, Expr, ExprKind, Package, PackageMetadata, PackedMode, VariantShape,
};
use crate::{DeclarationId, Primitive, RuntimeId, Sort, Term};
use std::collections::BTreeSet;
use std::fmt;

/// Domain separation prefix so `type_id` Merkle digests are isolated from any other hash.
const TYPE_ID_DOMAIN: &[u8] = b"flurry:type_id:v1\0";

/// Pure-Rust deterministic FIPS 180-4 SHA-256 implementation.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    pub fn new() -> Self {
        Self {
            state: Self::H0,
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        let mut rest = data;
        if self.block_len > 0 {
            let take = rest.len().min(64 - self.block_len);
            self.block[self.block_len..self.block_len + take].copy_from_slice(&rest[..take]);
            self.block_len += take;
            rest = &rest[take..];
            if self.block_len == 64 {
                let block = self.block;
                Self::compress(&mut self.state, &block);
                self.block_len = 0;
            }
        }
        while rest.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&rest[..64]);
            Self::compress(&mut self.state, &block);
            rest = &rest[64..];
        }
        if !rest.is_empty() {
            self.block[..rest.len()].copy_from_slice(rest);
            self.block_len = rest.len();
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            Self::compress(&mut self.state, &block);
            self.block_len = 0;
        }
        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        Self::compress(&mut self.state, &block);

        let mut out = [0u8; 32];
        for (idx, word) in self.state.iter().enumerate() {
            out[idx * 4..(idx + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (idx, slot) in w[..16].iter_mut().enumerate() {
            *slot = u32::from_be_bytes([
                block[idx * 4],
                block[idx * 4 + 1],
                block[idx * 4 + 2],
                block[idx * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(Self::K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

/// Domain-separated canonical stream writer backing each Merkle node digest.
#[derive(Clone, Debug)]
pub struct CanonicalDigest {
    sha: Sha256,
}

impl CanonicalDigest {
    pub fn new(node_tag: &[u8]) -> Self {
        let mut sha = Sha256::new();
        sha.update(TYPE_ID_DOMAIN);
        let mut digest = Self { sha };
        digest.bytes(node_tag);
        digest
    }

    pub fn tag(&mut self, tag: &[u8]) {
        self.bytes(tag);
    }

    pub fn u8(&mut self, value: u8) {
        self.sha.update(&[value]);
    }

    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub fn u16(&mut self, value: u16) {
        self.sha.update(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.sha.update(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.sha.update(&value.to_le_bytes());
    }

    pub fn u128(&mut self, value: u128) {
        self.sha.update(&value.to_le_bytes());
    }

    pub fn usize(&mut self, value: usize) {
        self.u64(value as u64);
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        self.usize(bytes.len());
        self.sha.update(bytes);
    }

    pub fn str(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    pub fn digest32(&mut self, digest: &[u8; 32]) {
        self.sha.update(digest);
    }

    pub fn finish(self) -> [u8; 32] {
        self.sha.finish()
    }
}

/// Deterministically map a 256-bit digest to a guaranteed non-zero 128-bit `type_id`.
pub fn digest_to_non_zero_u128(digest: [u8; 32]) -> u128 {
    let mut head = [0u8; 16];
    head.copy_from_slice(&digest[..16]);
    let raw = u128::from_be_bytes(head);
    if raw == 0 { 1 } else { raw }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConcreteStructInfo {
    pub packed: PackedMode,
    pub fields: Vec<(String, Term)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConcreteVariantInfo {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Vec<Term>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConcreteEnumInfo {
    pub variants: Vec<ConcreteVariantInfo>,
    pub encoding: Option<crate::enum_encoding::Representation>,
    pub symbol_encoding: Option<crate::enum_encoding::SymbolRepresentation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeIdError {
    MissingTarget,
    BudgetExhausted,
    NotAType(Term),
    OpenGeneric(Term),
    Unresolved(Term),
    ComptimeOnly(Term),
    NonGlobalRuntimeOrigin(RuntimeId),
    PendingDefinitionAttributes(DeclarationId),
    InvalidStableSince {
        declaration: Option<DeclarationId>,
        detail: String,
    },
    InvalidAbiMetadata {
        declaration: DeclarationId,
        detail: String,
    },
    Layout(LayoutError),
    Unsupported(Term),
    UnsupportedGlobalExpr(String),
}

impl fmt::Display for TypeIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTarget => f.write_str("(type-id-error missing-target)"),
            Self::BudgetExhausted => f.write_str("(type-id-error budget-exhausted)"),
            Self::NotAType(term) => write!(f, "(type-id-error (not-a-type {term}))"),
            Self::OpenGeneric(term) => write!(f, "(type-id-error (open-generic {term}))"),
            Self::Unresolved(term) => write!(f, "(type-id-error (unresolved {term}))"),
            Self::ComptimeOnly(term) => write!(f, "(type-id-error (comptime-only {term}))"),
            Self::NonGlobalRuntimeOrigin(id) => {
                write!(f, "(type-id-error (non-global-runtime-origin {id}))")
            }
            Self::PendingDefinitionAttributes(id) => {
                write!(f, "(type-id-error (pending-definition-attributes {id}))")
            }
            Self::InvalidStableSince {
                declaration: Some(id),
                detail,
            } => {
                write!(f, "(type-id-error (invalid-stable-since {id} ")?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
            Self::InvalidStableSince {
                declaration: None,
                detail,
            } => {
                f.write_str("(type-id-error (invalid-stable-since ")?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
            Self::InvalidAbiMetadata {
                declaration,
                detail,
            } => {
                write!(f, "(type-id-error (invalid-abi-metadata {declaration} ")?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
            Self::Layout(err) => write!(f, "(type-id-error (layout {err}))"),
            Self::Unsupported(term) => write!(f, "(type-id-error (unsupported {term}))"),
            Self::UnsupportedGlobalExpr(detail) => {
                f.write_str("(type-id-error (unsupported-global-expr ")?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
        }
    }
}

impl std::error::Error for TypeIdError {}

impl From<LayoutError> for TypeIdError {
    fn from(err: LayoutError) -> Self {
        match err {
            LayoutError::BudgetExhausted => Self::BudgetExhausted,
            LayoutError::ComptimeOnly(t) => Self::ComptimeOnly(t),
            LayoutError::Unresolved(t) => Self::Unresolved(t),
            other => Self::Layout(other),
        }
    }
}

impl From<EffectiveAttributeError> for TypeIdError {
    fn from(err: EffectiveAttributeError) -> Self {
        match err {
            EffectiveAttributeError::Pending(id) => Self::PendingDefinitionAttributes(id),
            EffectiveAttributeError::BudgetExhausted => Self::BudgetExhausted,
            EffectiveAttributeError::InvalidStableSince {
                declaration,
                detail,
            } => Self::InvalidStableSince {
                declaration,
                detail,
            },
            EffectiveAttributeError::MalformedObject(declaration) => Self::InvalidStableSince {
                declaration,
                detail: "malformed attribute object".into(),
            },
        }
    }
}

/// World queries needed to compute canonical 128-bit `type_id` fingerprints.
pub trait TypeIdWorld: LayoutWorld {
    fn package_metadata(&self, package: u64) -> Option<&PackageMetadata>;

    fn declaration_path(&self, declaration: DeclarationId) -> Option<&CanonicalDeclarationPath>;

    fn definition_attributes(&self) -> Option<&DefinitionAttributes>;

    fn global_runtime_origin(&self, id: RuntimeId) -> Option<&Expr>;

    fn nominal_name_fallback(&self, declaration: DeclarationId) -> Option<&str>;

    fn normal_form_name_fallback(&self, head: DeclarationId) -> Option<&str>;

    fn concrete_struct_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<ConcreteStructInfo>, LayoutError>;

    fn concrete_enum_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<ConcreteEnumInfo>, LayoutError>;
}

/// Standard `TypeIdWorld` implementation over an elaborated `nir::Package`.
pub struct PackageTypeIdWorld<'a>(pub &'a Package);

impl LayoutWorld for PackageTypeIdWorld<'_> {
    fn target(&self) -> Option<crate::target::TargetData> {
        PackageLayoutWorld(self.0).target()
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        PackageLayoutWorld(self.0).is_tuple_head(head)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        PackageLayoutWorld(self.0).is_list_head(head)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<crate::NominalKind> {
        PackageLayoutWorld(self.0).nominal_kind(ty)
    }

    fn is_template_identity(&self, ty: &Term) -> bool {
        PackageLayoutWorld(self.0).is_template_identity(ty)
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<crate::NominalLayout>, LayoutError> {
        PackageLayoutWorld(self.0).nominal_layout(ty, fuel)
    }

    fn nominal_field_index(
        &self,
        ty: &Term,
        field: &str,
        fuel: &mut usize,
    ) -> Result<crate::FieldLookup, LayoutError> {
        PackageLayoutWorld(self.0).nominal_field_index(ty, field, fuel)
    }
}

impl TypeIdWorld for PackageTypeIdWorld<'_> {
    fn package_metadata(&self, package: u64) -> Option<&PackageMetadata> {
        self.0.package_metadata.get(&package)
    }

    fn declaration_path(&self, declaration: DeclarationId) -> Option<&CanonicalDeclarationPath> {
        self.0.declaration_paths.get(&declaration)
    }

    fn definition_attributes(&self) -> Option<&DefinitionAttributes> {
        Some(&self.0.definition_attributes)
    }

    fn global_runtime_origin(&self, id: RuntimeId) -> Option<&Expr> {
        self.0.global_runtime_deliveries.get(&id)
    }

    fn nominal_name_fallback(&self, declaration: DeclarationId) -> Option<&str> {
        if let Some(def) = self.0.structs.iter().find(|s| s.declaration == declaration) {
            return Some(def.name.as_str());
        }
        if let Some(def) = self.0.enums.iter().find(|e| e.declaration == declaration) {
            return Some(def.name.as_str());
        }
        if let Some(def) = self
            .0
            .newtypes
            .iter()
            .find(|n| n.declaration == declaration)
        {
            return Some(def.name.as_str());
        }
        None
    }

    fn normal_form_name_fallback(&self, _head: DeclarationId) -> Option<&str> {
        None
    }

    fn concrete_struct_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<ConcreteStructInfo>, LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let Some(def) = self
            .0
            .structs
            .iter()
            .find(|s| s.declaration == *declaration)
        else {
            return Ok(None);
        };
        let field_types = crate::normal_forms::instantiate_value_fields_checked(
            &def.type_parameters,
            &def.parameter_types,
            arguments,
            def.fields.iter().map(|(_, t)| t),
            fuel,
        )
        .map_err(|err| match err {
            crate::NominalAdmissionError::Budget => LayoutError::BudgetExhausted,
            crate::NominalAdmissionError::Unresolved(_) => LayoutError::Unresolved(ty.clone()),
            crate::NominalAdmissionError::Invalid(_) => LayoutError::Unsupported(ty.clone()),
        })?;
        let fields = def
            .fields
            .iter()
            .map(|(name, _)| name.clone())
            .zip(field_types)
            .collect();
        Ok(Some(ConcreteStructInfo {
            packed: def.packed,
            fields,
        }))
    }

    fn concrete_enum_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<ConcreteEnumInfo>, LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let Some(def) = self.0.enums.iter().find(|e| e.declaration == *declaration) else {
            return Ok(None);
        };
        let mut variants = Vec::with_capacity(def.variants.len());
        for variant in &def.variants {
            let fields = crate::normal_forms::instantiate_value_fields_checked(
                &def.type_parameters,
                &def.parameter_types,
                arguments,
                variant.fields.iter(),
                fuel,
            )
            .map_err(|err| match err {
                crate::NominalAdmissionError::Budget => LayoutError::BudgetExhausted,
                crate::NominalAdmissionError::Unresolved(_) => LayoutError::Unresolved(ty.clone()),
                crate::NominalAdmissionError::Invalid(_) => LayoutError::Unsupported(ty.clone()),
            })?;
            variants.push(ConcreteVariantInfo {
                name: variant.name.clone(),
                shape: variant.shape.clone(),
                fields,
            });
        }
        Ok(Some(ConcreteEnumInfo {
            variants,
            encoding: def.encoding.clone(),
            symbol_encoding: def.symbol_encoding.clone(),
        }))
    }
}

/// Compute the deterministic 128-bit ABI `type_id` of `ty` within `package`.
pub fn compute_type_id(
    package: &Package,
    ty: &Term,
    fuel: &mut usize,
) -> Result<u128, TypeIdError> {
    compute_type_id_with_world(&PackageTypeIdWorld(package), ty, fuel)
}

/// Compute the deterministic 128-bit ABI `type_id` of `ty` using `world`.
pub fn compute_type_id_with_world(
    world: &impl TypeIdWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<u128, TypeIdError> {
    if world.target().is_none() {
        return Err(TypeIdError::MissingTarget);
    }
    if ty.sort() != Sort::Type {
        return Err(TypeIdError::NotAType(ty.clone()));
    }
    let mut engine = MerkleEngine {
        world,
        fuel,
        visiting: Vec::new(),
        visiting_runtime: BTreeSet::new(),
        closed_memo: Vec::new(),
    };
    let (digest, _) = engine.type_digest(ty, 0)?;
    Ok(digest_to_non_zero_u128(digest))
}

struct MerkleEngine<'a, W: TypeIdWorld> {
    world: &'a W,
    fuel: &'a mut usize,
    visiting: Vec<Term>,
    visiting_runtime: BTreeSet<RuntimeId>,
    closed_memo: Vec<(Term, [u8; 32])>,
}

impl<W: TypeIdWorld> MerkleEngine<'_, W> {
    fn charge(&mut self, cost: usize) -> Result<(), TypeIdError> {
        let Some(next) = self.fuel.checked_sub(cost) else {
            *self.fuel = 0;
            return Err(TypeIdError::BudgetExhausted);
        };
        *self.fuel = next;
        Ok(())
    }

    fn resolve_effective_version(
        &mut self,
        declaration: DeclarationId,
    ) -> Result<String, TypeIdError> {
        if let Some(attrs) = self.world.definition_attributes() {
            let resolved =
                EffectiveAttributes::resolve_declaration(attrs, Some(declaration), &[], self.fuel)?;
            if let Some(version) = resolved.stable_since()? {
                return Ok(version.to_string());
            }
        }
        let pkg = self
            .world
            .declaration_path(declaration)
            .map_or(declaration.package, |p| p.package);
        Ok(self
            .world
            .package_metadata(pkg)
            .map_or_else(|| "0.0.0".to_string(), |meta| meta.version.clone()))
    }

    fn hash_declaration_identity(
        &mut self,
        digest: &mut CanonicalDigest,
        declaration: DeclarationId,
    ) -> Result<(), TypeIdError> {
        let effective_version = self.resolve_effective_version(declaration)?;
        if let Some(path) = self.world.declaration_path(declaration) {
            digest.tag(b"canonical_decl");
            let pkg_name = self
                .world
                .package_metadata(path.package)
                .map_or("", |meta| meta.name.as_str());
            digest.str(pkg_name);
            digest.str(&effective_version);
            digest.usize(path.modules.len());
            for segment in &path.modules {
                digest.str(segment);
            }
            digest.str(&path.name);
        } else {
            // Synthetic NIR-only tests without resolved source module trees fall
            // back to package metadata and declaration name / local identity.
            digest.tag(b"fallback_decl");
            let pkg_name = self
                .world
                .package_metadata(declaration.package)
                .map_or("", |meta| meta.name.as_str());
            digest.str(pkg_name);
            digest.str(&effective_version);
            if let Some(name) = self
                .world
                .nominal_name_fallback(declaration)
                .or_else(|| self.world.normal_form_name_fallback(declaration))
            {
                digest.bool(true);
                digest.str(name);
                digest.u64(declaration.package);
                digest.u32(declaration.local);
            } else {
                digest.bool(false);
                digest.u64(declaration.package);
                digest.u32(declaration.local);
            }
        }
        if let Some(attrs) = self.world.definition_attributes() {
            match attrs.calling_convention(declaration, self.fuel) {
                Ok(None) => digest.u8(0),
                Ok(Some(CallingConvention::C)) => digest.u8(1),
                Ok(Some(CallingConvention::System)) => digest.u8(2),
                Ok(Some(CallingConvention::Stdcall)) => digest.u8(3),
                Ok(Some(CallingConvention::Win64)) => digest.u8(4),
                Ok(Some(CallingConvention::Sysv64)) => digest.u8(5),
                Err("ABI metadata is not available") => {
                    return Err(TypeIdError::PendingDefinitionAttributes(declaration));
                }
                Err("ABI metadata budget exhausted") => {
                    return Err(TypeIdError::BudgetExhausted);
                }
                Err(detail) => {
                    return Err(TypeIdError::InvalidAbiMetadata {
                        declaration,
                        detail: detail.to_string(),
                    });
                }
            }
        } else {
            digest.u8(0);
        }
        Ok(())
    }

    fn hash_layout_with_niche(digest: &mut CanonicalDigest, layout: &LayoutWithNiche) {
        digest.u64(layout.layout.size as u64);
        digest.u64(layout.layout.align as u64);
        match &layout.niche {
            None => digest.u8(0),
            Some(Niche {
                path,
                scalar,
                start,
                count,
                needs_bool_widen,
            }) => {
                digest.u8(1);
                digest.usize(path.len());
                for &idx in path {
                    digest.usize(idx);
                }
                match scalar {
                    NicheScalar::Pointer => digest.u8(0),
                    NicheScalar::Int { bits } => {
                        digest.u8(1);
                        digest.u32(*bits);
                    }
                }
                digest.u64(*start);
                digest.u64(*count);
                digest.bool(*needs_bool_widen);
            }
        }
    }

    /// Compute the Merkle 256-bit digest for a Type node, returning `(digest, min_escape_depth)`.
    /// `min_escape_depth == usize::MAX` means the subtree contains zero cycle back-edges to ancestors
    /// strictly above `self.visiting.len()`.
    fn type_digest(
        &mut self,
        ty: &Term,
        binder_depth: u32,
    ) -> Result<([u8; 32], usize), TypeIdError> {
        self.charge(1)?;
        if binder_depth == 0
            && self.visiting.is_empty()
            && let Some((_, cached)) = self.closed_memo.iter().find(|(k, _)| k == ty)
        {
            return Ok((*cached, usize::MAX));
        }

        if let Term::Nominal { .. } = ty
            && let Some(back_idx) = self.visiting.iter().position(|active| active == ty)
        {
            let mut d = CanonicalDigest::new(b"back_edge");
            // Encode both absolute preorder stack index and relative distance so
            // cycle topology is preserved unambiguously.
            d.usize(back_idx);
            d.usize(self.visiting.len() - back_idx);
            return Ok((d.finish(), back_idx));
        }

        match ty {
            Term::Rigid { .. } => return Err(TypeIdError::OpenGeneric(ty.clone())),
            Term::Hole { .. }
            | Term::AssociatedType { .. }
            | Term::AssociatedTypeCall(_)
            | Term::ListConcat { .. } => {
                return Err(TypeIdError::Unresolved(ty.clone()));
            }
            Term::Bound { depth, .. } => {
                if *depth >= binder_depth {
                    return Err(TypeIdError::OpenGeneric(ty.clone()));
                }
                return Err(TypeIdError::Unsupported(ty.clone()));
            }
            _ => {}
        }

        // Verify target layout availability up front for every non-cycle concrete Type.
        let target = self.world.target().ok_or(TypeIdError::MissingTarget)?;
        let layout = compute_layout_and_niche(self.world, ty, self.fuel)?;

        let mut min_escape = usize::MAX;
        let mut digest = CanonicalDigest::new(b"type");
        digest.u32(target.pointer_bits());
        Self::hash_layout_with_niche(&mut digest, &layout);

        match ty {
            Term::Primitive(prim) => {
                self.hash_primitive(&mut digest, *prim)?;
            }
            Term::Pointer(pointee) => {
                digest.tag(b"pointer");
                let (child, escape) = self.type_digest(pointee, binder_depth)?;
                min_escape = min_escape.min(escape);
                digest.digest32(&child);
            }
            Term::Optional(inner) => {
                digest.tag(b"optional");
                let (child, escape) = self.type_digest(inner, binder_depth)?;
                min_escape = min_escape.min(escape);
                digest.digest32(&child);
            }
            Term::ErrorResult { errors, result } => {
                digest.tag(b"error_result");
                let mut error_digests = BTreeSet::new();
                for err_ty in errors {
                    let (ed, escape) = self.type_digest(err_ty, binder_depth)?;
                    min_escape = min_escape.min(escape);
                    error_digests.insert(ed);
                }
                digest.usize(error_digests.len());
                for ed in error_digests {
                    digest.digest32(&ed);
                }
                let (rd, escape) = self.type_digest(result, binder_depth)?;
                min_escape = min_escape.min(escape);
                digest.digest32(&rd);
            }
            Term::Tuple(fields) => {
                digest.tag(b"tuple");
                let esc = self.hash_sequential_fields(
                    &mut digest,
                    None,
                    fields.iter().map(|t| (None, t)),
                    false,
                    binder_depth,
                )?;
                min_escape = min_escape.min(esc);
            }
            Term::Record(fields) => {
                digest.tag(b"record");
                let esc = self.hash_sequential_fields(
                    &mut digest,
                    None,
                    fields.iter().map(|(name, t)| (Some(name.as_str()), t)),
                    false,
                    binder_depth,
                )?;
                min_escape = min_escape.min(esc);
            }
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } => {
                if self.world.is_tuple_head(*head) {
                    let elements =
                        crate::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                            self.world.is_list_head(id)
                        })
                        .ok_or_else(|| TypeIdError::Unresolved(ty.clone()))?;
                    digest.tag(b"nf_tuple");
                    self.hash_declaration_identity(&mut digest, *head)?;
                    let esc = self.hash_sequential_fields(
                        &mut digest,
                        None,
                        elements.iter().map(|t| (None, t)),
                        false,
                        binder_depth,
                    )?;
                    min_escape = min_escape.min(esc);
                } else {
                    return Err(TypeIdError::Unsupported(ty.clone()));
                }
            }
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let current_depth = self.visiting.len();
                self.visiting.push(ty.clone());
                let res = self.hash_nominal(&mut digest, ty, *declaration, arguments, binder_depth);
                self.visiting.pop();
                let esc = res?;
                // Any back-edge targeting `current_depth` or deeper is resolved within this node.
                if esc < current_depth {
                    min_escape = min_escape.min(esc);
                }
            }
            Term::TraitObject {
                interface,
                arguments,
            } => {
                digest.tag(b"trait_object");
                self.hash_declaration_identity(&mut digest, *interface)?;
                let esc = self.hash_term_arguments(&mut digest, arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::Closure {
                environment,
                signature,
            } => {
                digest.tag(b"closure");
                let esc = self.hash_sequential_fields(
                    &mut digest,
                    None,
                    environment.iter().map(|t| (None, t)),
                    false,
                    binder_depth,
                )?;
                min_escape = min_escape.min(esc);
                let (sig_digest, sig_esc) = self.type_digest(signature, binder_depth)?;
                min_escape = min_escape.min(sig_esc);
                digest.digest32(&sig_digest);
            }
            Term::ContextualClosure { context, signature } => {
                digest.tag(b"contextual_closure");
                let esc_ctx =
                    self.hash_value_or_context_term(&mut digest, context, binder_depth)?;
                min_escape = min_escape.min(esc_ctx);
                let (sig_digest, sig_esc) = self.type_digest(signature, binder_depth)?;
                min_escape = min_escape.min(sig_esc);
                digest.digest32(&sig_digest);
            }
            Term::Function { parameters, result } => {
                digest.tag(b"fn");
                digest.usize(parameters.len());
                let next_depth = binder_depth.saturating_add(1);
                for param in parameters {
                    self.hash_sort(&mut digest, &param.sort);
                    let esc = if param.ty.sort() == Sort::Type {
                        digest.u8(1);
                        let (pd, pe) = self.type_digest(&param.ty, binder_depth)?;
                        digest.digest32(&pd);
                        pe
                    } else {
                        digest.u8(0);
                        self.hash_value_or_context_term(&mut digest, &param.ty, binder_depth)?
                    };
                    min_escape = min_escape.min(esc);
                }
                let (res_d, res_esc) = self.type_digest(result, next_depth)?;
                min_escape = min_escape.min(res_esc);
                digest.digest32(&res_d);
            }
            Term::UnsafeFunction(inner) => {
                digest.tag(b"unsafe_fn");
                let (inner_d, esc) = self.type_digest(inner, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&inner_d);
            }
            Term::PureFunction(inner) => {
                digest.tag(b"pure_fn");
                let (inner_d, esc) = self.type_digest(inner, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&inner_d);
            }
            Term::Computation { effects, result } => {
                digest.tag(b"computation");
                digest.usize(effects.len());
                for op in effects {
                    self.hash_declaration_identity(&mut digest, op.declaration)?;
                    let esc = self.hash_term_arguments(&mut digest, &op.arguments, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
                let (res_d, esc) = self.type_digest(result, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&res_d);
            }
            Term::Rigid { .. } => return Err(TypeIdError::OpenGeneric(ty.clone())),
            Term::Hole { .. }
            | Term::AssociatedType { .. }
            | Term::AssociatedTypeCall(_)
            | Term::ListConcat { .. } => {
                return Err(TypeIdError::Unresolved(ty.clone()));
            }
            Term::Bound { depth, .. } => {
                if *depth >= binder_depth {
                    return Err(TypeIdError::OpenGeneric(ty.clone()));
                }
                return Err(TypeIdError::Unsupported(ty.clone()));
            }
            _ => return Err(TypeIdError::Unsupported(ty.clone())),
        }

        let finished = digest.finish();
        if binder_depth == 0
            && self.visiting.is_empty()
            && min_escape == usize::MAX
            && self.closed_memo.len() < 256
        {
            self.closed_memo.push((ty.clone(), finished));
        }
        Ok((finished, min_escape))
    }

    fn hash_primitive(
        &self,
        digest: &mut CanonicalDigest,
        prim: Primitive,
    ) -> Result<(), TypeIdError> {
        digest.tag(b"primitive");
        match prim {
            Primitive::BitVec(width) => {
                digest.u8(1);
                digest.usize(width);
            }
            Primitive::Bool => digest.u8(2),
            Primitive::Char => digest.u8(3),
            Primitive::Str => digest.u8(4),
            Primitive::Unit => digest.u8(5),
            Primitive::Void => digest.u8(6),
            Primitive::Never => digest.u8(7),
            Primitive::I8 => digest.u8(8),
            Primitive::I16 => digest.u8(9),
            Primitive::I32 => digest.u8(10),
            Primitive::I64 => digest.u8(11),
            Primitive::I128 => digest.u8(12),
            Primitive::U8 => digest.u8(13),
            Primitive::U16 => digest.u8(14),
            Primitive::U32 => digest.u8(15),
            Primitive::U64 => digest.u8(16),
            Primitive::U128 => digest.u8(17),
            Primitive::Isize => digest.u8(18),
            Primitive::Usize => digest.u8(19),
            Primitive::F16 => digest.u8(20),
            Primitive::Bf16 => digest.u8(21),
            Primitive::F32 => digest.u8(22),
            Primitive::F64 => digest.u8(23),
            Primitive::Continuation => digest.u8(24),
            Primitive::Type
            | Primitive::Object
            | Primitive::Symbol
            | Primitive::Integer
            | Primitive::Float => return Err(TypeIdError::ComptimeOnly(Term::Primitive(prim))),
        }
        Ok(())
    }

    fn hash_sort(&self, digest: &mut CanonicalDigest, sort: &Sort) {
        match sort {
            Sort::Type => digest.u8(0),
            Sort::ComptimeValue => digest.u8(1),
            Sort::Runtime => digest.u8(2),
        }
    }

    fn hash_nominal(
        &mut self,
        digest: &mut CanonicalDigest,
        ty: &Term,
        declaration: DeclarationId,
        arguments: &[Term],
        binder_depth: u32,
    ) -> Result<usize, TypeIdError> {
        let mut min_escape = usize::MAX;
        digest.tag(b"nominal");
        self.hash_declaration_identity(digest, declaration)?;
        let arg_esc = self.hash_term_arguments(digest, arguments, binder_depth)?;
        min_escape = min_escape.min(arg_esc);

        let kind = self
            .world
            .nominal_kind(ty)
            .ok_or_else(|| TypeIdError::Unresolved(ty.clone()))?;
        let layout_kind = self
            .world
            .nominal_layout(ty, self.fuel)?
            .ok_or_else(|| TypeIdError::Unresolved(ty.clone()))?;

        match kind {
            crate::NominalKind::Struct => {
                if let Some(info) = self.world.concrete_struct_info(ty, self.fuel)? {
                    match info.packed {
                        PackedMode::None | PackedMode::Byte => {
                            digest.u8(match info.packed {
                                PackedMode::None => 10,
                                PackedMode::Byte => 11,
                                PackedMode::Bit => unreachable!(),
                            });
                            let esc = self.hash_sequential_fields(
                                digest,
                                None,
                                info.fields.iter().map(|(n, t)| (Some(n.as_str()), t)),
                                info.packed == PackedMode::Byte,
                                binder_depth,
                            )?;
                            min_escape = min_escape.min(esc);
                        }
                        PackedMode::Bit => {
                            digest.u8(12);
                            let field_types: Vec<Term> =
                                info.fields.iter().map(|(_, t)| t.clone()).collect();
                            let bp = bit_packed_struct_layout(self.world, &field_types, self.fuel)
                                .map_err(Self::map_bit_packed_error)?;
                            digest.u32(bp.total_bits);
                            digest.usize(bp.fields.len());
                            for ((name, field_ty), bp_field) in info.fields.iter().zip(&bp.fields) {
                                digest.str(name);
                                digest.u32(bp_field.bit_offset);
                                digest.u32(bp_field.bit_width);
                                digest.bool(bp_field.restricted);
                                let (fd, esc) = self.type_digest(field_ty, binder_depth)?;
                                min_escape = min_escape.min(esc);
                                digest.digest32(&fd);
                            }
                        }
                    }
                } else {
                    // Fallback when a synthetic LayoutWorld provides only NominalLayout.
                    match layout_kind {
                        crate::NominalLayout::Struct(fields) => {
                            digest.u8(10);
                            let esc = self.hash_sequential_fields(
                                digest,
                                None,
                                fields.iter().map(|t| (None, t)),
                                false,
                                binder_depth,
                            )?;
                            min_escape = min_escape.min(esc);
                        }
                        crate::NominalLayout::PackedStruct(fields) => {
                            digest.u8(11);
                            let esc = self.hash_sequential_fields(
                                digest,
                                None,
                                fields.iter().map(|t| (None, t)),
                                true,
                                binder_depth,
                            )?;
                            min_escape = min_escape.min(esc);
                        }
                        crate::NominalLayout::BitPackedStruct(fields) => {
                            digest.u8(12);
                            let bp = bit_packed_struct_layout(self.world, &fields, self.fuel)
                                .map_err(Self::map_bit_packed_error)?;
                            digest.u32(bp.total_bits);
                            digest.usize(bp.fields.len());
                            for (field_ty, bp_field) in fields.iter().zip(&bp.fields) {
                                digest.u32(bp_field.bit_offset);
                                digest.u32(bp_field.bit_width);
                                digest.bool(bp_field.restricted);
                                let (fd, esc) = self.type_digest(field_ty, binder_depth)?;
                                min_escape = min_escape.min(esc);
                                digest.digest32(&fd);
                            }
                        }
                        _ => return Err(TypeIdError::Unsupported(ty.clone())),
                    }
                }
            }
            crate::NominalKind::Newtype => {
                digest.u8(20);
                let crate::NominalLayout::Newtype(repr) = layout_kind else {
                    return Err(TypeIdError::Unsupported(ty.clone()));
                };
                let repr_layout = compute_layout_and_niche(self.world, &repr, self.fuel)?;
                Self::hash_layout_with_niche(digest, &repr_layout);
                let (repr_digest, esc) = self.type_digest(&repr, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&repr_digest);
            }
            crate::NominalKind::Enum => {
                digest.u8(30);
                let enum_info = self.world.concrete_enum_info(ty, self.fuel)?;
                match layout_kind {
                    crate::NominalLayout::EnumSymbol => digest.u8(1),
                    crate::NominalLayout::EnumFixedBits {
                        width,
                        niche,
                        exhaustive,
                    } => {
                        digest.u8(2);
                        digest.u32(width);
                        digest.bool(exhaustive);
                        if let Some((lo, hi)) = niche {
                            digest.bool(true);
                            digest.u64(lo);
                            digest.u64(hi);
                        } else {
                            digest.bool(false);
                        }
                    }
                    crate::NominalLayout::EnumUnit(count) => {
                        digest.u8(3);
                        digest.usize(count);
                        digest.u32(unit_enum_tag_bits(count)?);
                    }
                    crate::NominalLayout::EnumAggregate(ref variants) => {
                        digest.u8(4);
                        digest.usize(variants.len());
                        digest.u32(sum_tag_bits(variants.len())?);
                        for (idx, fields) in variants.iter().enumerate() {
                            digest.usize(idx);
                            let esc = self.hash_sequential_fields(
                                digest,
                                None,
                                fields.iter().map(|t| (None, t)),
                                false,
                                binder_depth,
                            )?;
                            min_escape = min_escape.min(esc);
                        }
                    }
                    _ => return Err(TypeIdError::Unsupported(ty.clone())),
                }
                if let Some(info) = enum_info {
                    digest.bool(true);
                    if let Some(rep) = &info.encoding {
                        digest.bool(true);
                        digest.usize(rep.width());
                        if let Some(base) = rep.base_type {
                            self.hash_primitive(digest, base)?;
                        } else {
                            digest.u8(0);
                        }
                        digest.usize(rep.variants().len());
                        for v in rep.variants() {
                            digest.str(v.name());
                            digest.usize(v.width());
                            digest.usize(v.fixed_mask().words().len());
                            for &word in v.fixed_mask().words() {
                                digest.u32(word);
                            }
                            digest.usize(v.fixed_value().words().len());
                            for &word in v.fixed_value().words() {
                                digest.u32(word);
                            }
                            digest.usize(v.fields().len());
                            for f in v.fields() {
                                digest.str(&f.name);
                                digest.usize(f.start);
                                digest.usize(f.width);
                            }
                        }
                    } else {
                        digest.bool(false);
                    }
                    if let Some(sym) = &info.symbol_encoding {
                        digest.bool(true);
                        digest.usize(sym.variants().len());
                        for sv in sym.variants() {
                            digest.str(&sv.name);
                            digest.str(&sv.value);
                        }
                    } else {
                        digest.bool(false);
                    }
                    digest.usize(info.variants.len());
                    for (idx, variant) in info.variants.iter().enumerate() {
                        digest.usize(idx);
                        digest.str(&variant.name);
                        match &variant.shape {
                            VariantShape::Unit => digest.u8(0),
                            VariantShape::Tuple => digest.u8(1),
                            VariantShape::Named(names) => {
                                digest.u8(2);
                                digest.usize(names.len());
                                for n in names {
                                    digest.str(n);
                                }
                            }
                        }
                        let esc = self.hash_sequential_fields(
                            digest,
                            None,
                            variant.fields.iter().map(|t| (None, t)),
                            false,
                            binder_depth,
                        )?;
                        min_escape = min_escape.min(esc);
                    }
                } else {
                    digest.bool(false);
                }
            }
        }
        Ok(min_escape)
    }

    fn hash_sequential_fields<'b>(
        &mut self,
        digest: &mut CanonicalDigest,
        initial_offset: Option<usize>,
        fields: impl Iterator<Item = (Option<&'b str>, &'b Term)>,
        packed_byte: bool,
        binder_depth: u32,
    ) -> Result<usize, TypeIdError> {
        let collected: Vec<(Option<&str>, &Term)> = fields.collect();
        digest.usize(collected.len());
        digest.bool(packed_byte);
        let mut min_escape = usize::MAX;
        let mut cursor = initial_offset.unwrap_or(0) as u64;
        for (name, field_ty) in collected {
            let field_layout = compute_layout_and_niche(self.world, field_ty, self.fuel)?;
            let offset = if packed_byte {
                cursor
            } else {
                align_up(cursor, field_layout.layout.align)?
            };
            cursor = offset
                .checked_add(field_layout.layout.size)
                .ok_or(TypeIdError::Layout(LayoutError::Overflow))?;
            if let Some(name) = name {
                digest.bool(true);
                digest.str(name);
            } else {
                digest.bool(false);
            }
            digest.u64(offset);
            Self::hash_layout_with_niche(digest, &field_layout);
            let (fd, esc) = self.type_digest(field_ty, binder_depth)?;
            min_escape = min_escape.min(esc);
            digest.digest32(&fd);
        }
        Ok(min_escape)
    }

    fn hash_term_arguments(
        &mut self,
        digest: &mut CanonicalDigest,
        arguments: &[Term],
        binder_depth: u32,
    ) -> Result<usize, TypeIdError> {
        let mut min_escape = usize::MAX;
        digest.usize(arguments.len());
        for arg in arguments {
            self.hash_sort(digest, &arg.sort());
            if arg.sort() == Sort::Type {
                digest.u8(1);
                let (d, esc) = self.type_digest(arg, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&d);
            } else {
                digest.u8(2);
                let esc = self.hash_value_or_context_term(digest, arg, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
        }
        Ok(min_escape)
    }

    fn hash_value_or_context_term(
        &mut self,
        digest: &mut CanonicalDigest,
        term: &Term,
        binder_depth: u32,
    ) -> Result<usize, TypeIdError> {
        self.charge(1)?;
        let mut min_escape = usize::MAX;
        match term {
            Term::Unit => digest.tag(b"v_unit"),
            Term::Bool(v) => {
                digest.tag(b"v_bool");
                digest.bool(*v);
            }
            Term::Char(c) => {
                digest.tag(b"v_char");
                digest.u32(*c as u32);
            }
            Term::BitVec(bv) => {
                digest.tag(b"v_bitvec");
                digest.usize(bv.width());
                digest.usize(bv.words().len());
                for &w in bv.words() {
                    digest.u32(w);
                }
            }
            Term::Float16(bits) => {
                digest.tag(b"v_f16");
                digest.u16(*bits);
            }
            Term::BFloat16(bits) => {
                digest.tag(b"v_bf16");
                digest.u16(*bits);
            }
            Term::Float32(bits) => {
                digest.tag(b"v_f32");
                digest.u32(*bits);
            }
            Term::Float64(bits) => {
                digest.tag(b"v_f64");
                digest.u64(*bits);
            }
            Term::String(s) => {
                digest.tag(b"v_str");
                digest.str(s);
            }
            Term::Symbol(s) => {
                digest.tag(b"v_sym");
                digest.str(s);
            }
            Term::Integer(int) => {
                digest.tag(b"v_int");
                digest.str(&int.to_string());
            }
            Term::Float(dec) => {
                digest.tag(b"v_dec");
                digest.str(&dec.to_string());
            }
            Term::TupleValue(items) | Term::ClosureContextValue(items) => {
                digest.tag(if matches!(term, Term::TupleValue(_)) {
                    b"v_tuple"
                } else {
                    b"v_ctx"
                });
                digest.usize(items.len());
                for item in items {
                    let esc = self.hash_value_or_context_term(digest, item, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
            }
            Term::RecordValue(fields) => {
                digest.tag(b"v_record");
                digest.usize(fields.len());
                for (name, val) in fields {
                    digest.str(name);
                    let esc = self.hash_value_or_context_term(digest, val, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
            }
            Term::NominalValue {
                ty,
                variant,
                fields,
            } => {
                digest.tag(b"v_nominal");
                if ty.sort() == Sort::Type
                    && !matches!(
                        ty.as_ref(),
                        Term::Primitive(Primitive::Object | Primitive::Type)
                    )
                {
                    digest.u8(1);
                    let (td, esc) = self.type_digest(ty, binder_depth)?;
                    min_escape = min_escape.min(esc);
                    digest.digest32(&td);
                } else {
                    digest.u8(0);
                    self.hash_comptime_domain(digest, ty)?;
                }
                match variant {
                    Some(idx) => {
                        digest.bool(true);
                        digest.u32(*idx);
                    }
                    None => digest.bool(false),
                }
                digest.usize(fields.len());
                for field in fields {
                    let esc = self.hash_value_or_context_term(digest, field, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
            }
            Term::OptionalValue { ty, value } => {
                digest.tag(b"v_opt");
                let (td, esc) = self.type_digest(ty, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&td);
                match value {
                    Some(inner) => {
                        digest.bool(true);
                        let esc = self.hash_value_or_context_term(digest, inner, binder_depth)?;
                        min_escape = min_escape.min(esc);
                    }
                    None => digest.bool(false),
                }
            }
            Term::ErrorValue { ty, label, value } => {
                digest.tag(b"v_err");
                let (td, esc) = self.type_digest(ty, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&td);
                match label {
                    Some(lbl) => {
                        digest.bool(true);
                        let (ld, esc) = self.type_digest(lbl, binder_depth)?;
                        min_escape = min_escape.min(esc);
                        digest.digest32(&ld);
                    }
                    None => digest.bool(false),
                }
                let esc = self.hash_value_or_context_term(digest, value, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::NormalForm {
                head,
                sort,
                arguments,
            } => {
                digest.tag(b"v_nf");
                self.hash_declaration_identity(digest, *head)?;
                self.hash_sort(digest, sort);
                let esc = self.hash_term_arguments(digest, arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::FunctionValue {
                declaration,
                arguments,
            } => {
                digest.tag(b"v_fn");
                self.hash_declaration_identity(digest, *declaration)?;
                let esc = self.hash_term_arguments(digest, arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::ClosureSnapshot { factory, arguments } => {
                digest.tag(b"v_closure_snap");
                self.hash_declaration_identity(digest, *factory)?;
                let esc = self.hash_term_arguments(digest, arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::ClosureValue { ty, body, captures } => {
                digest.tag(b"v_closure");
                let (td, esc) = self.type_digest(ty, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&td);
                digest.str(body);
                digest.usize(captures.len());
                for cap in captures {
                    let esc = self.hash_value_or_context_term(digest, cap, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
            }
            Term::MethodValue(method) => {
                digest.tag(b"v_method");
                self.hash_declaration_identity(digest, method.interface.declaration)?;
                let esc =
                    self.hash_term_arguments(digest, &method.interface.arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
                let (sd, s_esc) = self.type_digest(&method.subject, binder_depth)?;
                min_escape = min_escape.min(s_esc);
                digest.digest32(&sd);
                self.hash_declaration_identity(digest, method.member)?;
                let m_esc =
                    self.hash_term_arguments(digest, &method.method_arguments, binder_depth)?;
                min_escape = min_escape.min(m_esc);
            }
            Term::Lift(inner) => {
                digest.tag(b"v_lift");
                let (td, esc) = self.type_digest(inner, binder_depth)?;
                min_escape = min_escape.min(esc);
                digest.digest32(&td);
            }
            Term::Runtime(_) | Term::RuntimeField { .. } => {
                let esc = self.hash_runtime_origin(digest, term, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            Term::Bound { depth, index, sort } => {
                if *depth >= binder_depth {
                    return Err(TypeIdError::OpenGeneric(term.clone()));
                }
                digest.tag(b"v_bound");
                digest.u32(*depth);
                digest.usize(*index);
                self.hash_sort(digest, sort);
            }
            Term::Rigid { .. } => return Err(TypeIdError::OpenGeneric(term.clone())),
            Term::Hole { .. }
            | Term::ListConcat { .. }
            | Term::AssociatedType { .. }
            | Term::AssociatedTypeCall(_) => {
                return Err(TypeIdError::Unresolved(term.clone()));
            }
            _ => return Err(TypeIdError::Unsupported(term.clone())),
        }
        Ok(min_escape)
    }

    fn hash_comptime_domain(
        &mut self,
        digest: &mut CanonicalDigest,
        domain: &Term,
    ) -> Result<(), TypeIdError> {
        digest.tag(b"ct_domain");
        match domain {
            Term::Primitive(p) => {
                digest.u8(1);
                digest.str(&p.to_string());
            }
            Term::NormalForm {
                head,
                sort,
                arguments,
            } => {
                digest.u8(2);
                self.hash_declaration_identity(digest, *head)?;
                self.hash_sort(digest, sort);
                self.hash_term_arguments(digest, arguments, 0)?;
            }
            Term::ClosureContextType => digest.u8(3),
            Term::EffectRowType => digest.u8(4),
            Term::ErrorRowType => digest.u8(5),
            Term::ReachabilityType => digest.u8(6),
            Term::AnyType => digest.u8(7),
            Term::ComptimeFunction(inner) => {
                digest.u8(8);
                self.hash_comptime_domain(digest, inner)?;
            }
            Term::NormalFormFunction(inner, collected) => {
                digest.u8(10);
                match collected {
                    Some(idx) => {
                        digest.bool(true);
                        digest.usize(*idx);
                    }
                    None => digest.bool(false),
                }
                self.hash_comptime_domain(digest, inner)?;
            }
            Term::Function { parameters, result } => {
                digest.u8(11);
                digest.usize(parameters.len());
                for param in parameters {
                    self.hash_sort(digest, &param.sort);
                    self.hash_comptime_domain(digest, &param.ty)?;
                }
                self.hash_comptime_domain(digest, result)?;
            }
            Term::UnsafeFunction(inner) => {
                digest.u8(12);
                self.hash_comptime_domain(digest, inner)?;
            }
            Term::PureFunction(inner) => {
                digest.u8(14);
                self.hash_comptime_domain(digest, inner)?;
            }
            Term::Bound { depth, index, sort } => {
                digest.u8(13);
                digest.u32(*depth);
                digest.usize(*index);
                self.hash_sort(digest, sort);
            }
            Term::Rigid { .. } => return Err(TypeIdError::OpenGeneric(domain.clone())),
            Term::Hole { .. }
            | Term::AssociatedType { .. }
            | Term::AssociatedTypeCall(_)
            | Term::ListConcat { .. } => {
                return Err(TypeIdError::Unresolved(domain.clone()));
            }
            other => {
                let (d, _) = self.type_digest(other, 0)?;
                digest.u8(9);
                digest.digest32(&d);
            }
        }
        Ok(())
    }

    fn map_bit_packed_error(err: crate::BitPackedError) -> TypeIdError {
        match err {
            crate::BitPackedError::Layout(layout_err) => TypeIdError::from(layout_err),
            crate::BitPackedError::WidthOverflow => TypeIdError::Layout(LayoutError::Overflow),
            crate::BitPackedError::UnsupportedField { ty, .. } => TypeIdError::Unsupported(ty),
        }
    }

    fn hash_runtime_origin(
        &mut self,
        digest: &mut CanonicalDigest,
        term: &Term,
        binder_depth: u32,
    ) -> Result<usize, TypeIdError> {
        let mut indices = Vec::new();
        let mut cursor = term;
        while let Term::RuntimeField { base, index } = cursor {
            self.charge(1)?;
            indices.push(*index);
            cursor = base;
        }
        let Term::Runtime(id) = cursor else {
            return Err(TypeIdError::Unsupported(term.clone()));
        };
        let Some(expr) = self.world.global_runtime_origin(*id).cloned() else {
            return Err(TypeIdError::NonGlobalRuntimeOrigin(*id));
        };
        if !self.visiting_runtime.insert(*id) {
            return Err(TypeIdError::NonGlobalRuntimeOrigin(*id));
        }
        digest.tag(b"global_runtime_origin");
        let mut bound_locals = std::collections::BTreeMap::new();
        let res = self.hash_global_expr(digest, &expr, *id, binder_depth, &mut bound_locals);
        self.visiting_runtime.remove(id);
        let min_escape = res?;
        digest.usize(indices.len());
        for index in indices.into_iter().rev() {
            digest.usize(index);
        }
        Ok(min_escape)
    }

    fn hash_global_expr(
        &mut self,
        digest: &mut CanonicalDigest,
        expr: &Expr,
        origin_id: RuntimeId,
        binder_depth: u32,
        bound_locals: &mut std::collections::BTreeMap<crate::program::LocalId, usize>,
    ) -> Result<usize, TypeIdError> {
        self.charge(1)?;
        let mut min_escape = usize::MAX;
        digest.tag(b"global_expr");
        if matches!(expr.kind, ExprKind::TraitMethod { .. }) {
            // TraitMethod.ty carries the un-projected trait signature (e.g., Self.Target);
            // the enclosing Call.ty already holds the resolved result type.
            digest.u8(3);
        } else if expr.ty.sort() == Sort::Type
            && !matches!(
                expr.ty,
                Term::Primitive(Primitive::Type | Primitive::Object)
                    | Term::ClosureContextType
                    | Term::EffectRowType
                    | Term::ErrorRowType
                    | Term::ReachabilityType
                    | Term::AnyType
                    | Term::ComptimeFunction(_)
                    | Term::NormalFormFunction(..)
                    | Term::Function { .. }
                    | Term::UnsafeFunction(_)
                    | Term::PureFunction(_)
            )
        {
            digest.u8(1);
            let (td, esc) = self.type_digest(&expr.ty, binder_depth)?;
            min_escape = min_escape.min(esc);
            digest.digest32(&td);
        } else {
            digest.u8(2);
            self.hash_comptime_domain(digest, &expr.ty)?;
        }
        match &expr.kind {
            ExprKind::Local(local) => {
                let Some(&canonical_index) = bound_locals.get(local) else {
                    return Err(TypeIdError::NonGlobalRuntimeOrigin(origin_id));
                };
                digest.tag(b"bound_local");
                digest.usize(canonical_index);
            }
            ExprKind::ConstAddress { declaration, value } => {
                digest.tag(b"const_addr");
                self.hash_declaration_identity(digest, *declaration)?;
                let esc =
                    self.hash_global_expr(digest, value, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::StaticAddress(declaration) => {
                digest.tag(b"static_addr");
                self.hash_declaration_identity(digest, *declaration)?;
            }
            ExprKind::AddressOf(inner) => {
                digest.tag(b"addr_of");
                let esc =
                    self.hash_global_expr(digest, inner, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::ReadPointer(inner) => {
                digest.tag(b"read_ptr");
                let esc =
                    self.hash_global_expr(digest, inner, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::Field { value, index, .. } => {
                digest.tag(b"field");
                digest.usize(*index);
                let esc =
                    self.hash_global_expr(digest, value, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::AnonymousField { value, index, .. } => {
                digest.tag(b"anon_field");
                digest.usize(*index);
                let esc =
                    self.hash_global_expr(digest, value, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::PointerField { pointer, index, .. } => {
                digest.tag(b"ptr_field");
                digest.usize(*index);
                let esc =
                    self.hash_global_expr(digest, pointer, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::PointerAnonymousField { pointer, index, .. } => {
                digest.tag(b"ptr_anon_field");
                digest.usize(*index);
                let esc =
                    self.hash_global_expr(digest, pointer, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::Function(declaration) => {
                digest.tag(b"fn_ref");
                self.hash_declaration_identity(digest, *declaration)?;
            }
            ExprKind::FunctionInstance {
                declaration,
                arguments,
            } => {
                digest.tag(b"fn_inst");
                self.hash_declaration_identity(digest, *declaration)?;
                let esc = self.hash_term_arguments(digest, arguments, binder_depth)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::TraitMethod {
                method_arguments,
                interface_arguments,
                interface,
                member,
                subject,
            } => {
                digest.tag(b"trait_method");
                self.hash_declaration_identity(digest, *interface)?;
                self.hash_declaration_identity(digest, *member)?;
                if subject.sort() == Sort::Type {
                    digest.u8(1);
                    let (sd, esc) = self.type_digest(subject, binder_depth)?;
                    min_escape = min_escape.min(esc);
                    digest.digest32(&sd);
                } else {
                    digest.u8(0);
                    let esc = self.hash_value_or_context_term(digest, subject, binder_depth)?;
                    min_escape = min_escape.min(esc);
                }
                let esc_i = self.hash_term_arguments(digest, interface_arguments, binder_depth)?;
                min_escape = min_escape.min(esc_i);
                let esc_m = self.hash_term_arguments(digest, method_arguments, binder_depth)?;
                min_escape = min_escape.min(esc_m);
            }
            ExprKind::Call {
                callee,
                arguments,
                parameter_bindings,
                env_arguments,
            } => {
                digest.tag(b"expr_call");
                let esc_callee =
                    self.hash_global_expr(digest, callee, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc_callee);
                let saved_len = bound_locals.len();
                let mut inserted = Vec::with_capacity(parameter_bindings.len());
                digest.usize(parameter_bindings.len());
                for (offset, binding) in parameter_bindings.iter().enumerate() {
                    let canonical = saved_len + offset;
                    digest.usize(canonical);
                    let prev = bound_locals.insert(*binding, canonical);
                    inserted.push((*binding, prev));
                }
                digest.usize(arguments.len());
                for arg in arguments {
                    digest.usize(arg.parameter);
                    digest.bool(arg.is_default);
                    digest.str(&arg.passing.to_string());
                    let esc = self.hash_global_expr(
                        digest,
                        &arg.value,
                        origin_id,
                        binder_depth,
                        bound_locals,
                    )?;
                    min_escape = min_escape.min(esc);
                }
                for (binding, prev) in inserted.into_iter().rev() {
                    match prev {
                        Some(old) => {
                            bound_locals.insert(binding, old);
                        }
                        None => {
                            bound_locals.remove(&binding);
                        }
                    }
                }
                digest.usize(env_arguments.len());
                for env_arg in env_arguments {
                    let esc = self.hash_global_expr(
                        digest,
                        env_arg,
                        origin_id,
                        binder_depth,
                        bound_locals,
                    )?;
                    min_escape = min_escape.min(esc);
                }
            }
            ExprKind::CallPreparation { statements, result } => {
                digest.tag(b"expr_call_prep");
                let mut inserted = Vec::new();
                digest.usize(statements.len());
                for stmt in statements {
                    if let ExprKind::Let { binding, value } = &stmt.kind {
                        digest.tag(b"prep_let");
                        let esc = self.hash_global_expr(
                            digest,
                            value,
                            origin_id,
                            binder_depth,
                            bound_locals,
                        )?;
                        min_escape = min_escape.min(esc);
                        let canonical = bound_locals.len();
                        digest.usize(canonical);
                        let prev = bound_locals.insert(*binding, canonical);
                        inserted.push((*binding, prev));
                    } else {
                        let esc = self.hash_global_expr(
                            digest,
                            stmt,
                            origin_id,
                            binder_depth,
                            bound_locals,
                        )?;
                        min_escape = min_escape.min(esc);
                    }
                }
                match result {
                    Some(res_expr) => {
                        digest.bool(true);
                        let esc = self.hash_global_expr(
                            digest,
                            res_expr,
                            origin_id,
                            binder_depth,
                            bound_locals,
                        )?;
                        min_escape = min_escape.min(esc);
                    }
                    None => digest.bool(false),
                }
                for (binding, prev) in inserted.into_iter().rev() {
                    match prev {
                        Some(old) => {
                            bound_locals.insert(binding, old);
                        }
                        None => {
                            bound_locals.remove(&binding);
                        }
                    }
                }
            }
            ExprKind::TypeValue(term)
            | ExprKind::ComptimeValue(term)
            | ExprKind::StagedValue(term) => {
                digest.tag(b"term_val");
                let esc = if term.sort() == Sort::Type {
                    let (d, e) = self.type_digest(term, binder_depth)?;
                    digest.digest32(&d);
                    e
                } else {
                    self.hash_value_or_context_term(digest, term, binder_depth)?
                };
                min_escape = min_escape.min(esc);
            }
            ExprKind::Unit => digest.tag(b"expr_unit"),
            ExprKind::Variant {
                declaration,
                index,
                fields,
            } => {
                digest.tag(b"expr_variant");
                self.hash_declaration_identity(digest, *declaration)?;
                digest.usize(*index);
                digest.usize(fields.len());
                for f in fields {
                    let esc =
                        self.hash_global_expr(digest, f, origin_id, binder_depth, bound_locals)?;
                    min_escape = min_escape.min(esc);
                }
            }
            ExprKind::Tuple(fields) => {
                digest.tag(b"expr_tuple");
                digest.usize(fields.len());
                for f in fields {
                    let esc =
                        self.hash_global_expr(digest, f, origin_id, binder_depth, bound_locals)?;
                    min_escape = min_escape.min(esc);
                }
            }
            ExprKind::Record {
                declaration,
                fields,
            } => {
                digest.tag(b"expr_record_nom");
                self.hash_declaration_identity(digest, *declaration)?;
                digest.usize(fields.len());
                for f in fields {
                    let esc =
                        self.hash_global_expr(digest, f, origin_id, binder_depth, bound_locals)?;
                    min_escape = min_escape.min(esc);
                }
            }
            ExprKind::AnonymousRecord { fields } => {
                digest.tag(b"expr_record");
                digest.usize(fields.len());
                for f in fields {
                    let esc =
                        self.hash_global_expr(digest, f, origin_id, binder_depth, bound_locals)?;
                    min_escape = min_escape.min(esc);
                }
            }
            ExprKind::OptionalAbsent => digest.tag(b"expr_opt_absent"),
            ExprKind::OptionalPresent(inner) => {
                digest.tag(b"expr_opt_present");
                let esc =
                    self.hash_global_expr(digest, inner, origin_id, binder_depth, bound_locals)?;
                min_escape = min_escape.min(esc);
            }
            ExprKind::Literal(lit) => {
                digest.tag(b"expr_lit");
                digest.str(&lit.to_string());
            }
            other => {
                return Err(TypeIdError::UnsupportedGlobalExpr(other.to_string()));
            }
        }
        Ok(min_escape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normal_forms::Head;
    use crate::program::{Enum, SourceRange, Struct, Variant};
    use crate::target::TargetData;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    fn decl(package: u64, local: u32) -> DeclarationId {
        DeclarationId { package, local }
    }

    fn base_package(pointer_bits: u32, version: &str) -> Package {
        let mut pkg = Package::default();
        pkg.target = Some(TargetData::new(pointer_bits).unwrap());
        pkg.package_metadata.insert(
            1,
            PackageMetadata {
                name: "acme".into(),
                version: version.into(),
            },
        );
        pkg
    }

    fn stable_since_attr(version: &str) -> Arc<Term> {
        Arc::new(crate::objects::value(vec![crate::objects::entry(
            Some(crate::program::STABLE_SINCE_KEY),
            Term::Primitive(Primitive::Str),
            Term::String(version.into()),
        )]))
    }

    #[test]
    fn sha256_matches_nist_test_vectors() {
        let hex =
            |bytes: [u8; 32]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
        assert_eq!(
            hex(Sha256::new().finish()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let mut s = Sha256::new();
        s.update(b"abc");
        assert_eq!(
            hex(s.finish()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut multi = Sha256::new();
        multi.update(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
        assert_eq!(
            hex(multi.finish()),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn zero_digest_maps_deterministically_to_non_zero() {
        assert_eq!(digest_to_non_zero_u128([0u8; 32]), 1u128);
        let mut non_zero = [0u8; 32];
        non_zero[15] = 42;
        assert_eq!(digest_to_non_zero_u128(non_zero), 42u128);
    }

    #[test]
    fn cross_version_stable_since_preserves_type_id_even_if_local_ids_shift() {
        let mut pkg_v1 = base_package(64, "1.0.0");
        let id_v1 = decl(1, 5);
        pkg_v1.structs.push(Struct {
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![decl(1, 6)],
            declaration: id_v1,
            name: "Buffer".into(),
            fields: vec![
                ("len".into(), Term::Primitive(Primitive::Usize)),
                (
                    "data".into(),
                    Term::Pointer(Box::new(Term::Rigid {
                        declaration: decl(1, 6),
                        sort: Sort::Type,
                    })),
                ),
            ],
            packed: PackedMode::None,
        });
        pkg_v1.declaration_paths.insert(
            id_v1,
            CanonicalDeclarationPath {
                package: 1,
                modules: vec!["collections".into()],
                name: "Buffer".into(),
            },
        );
        pkg_v1.definition_attributes = DefinitionAttributes::Frozen(Arc::new(BTreeMap::from([(
            id_v1,
            stable_since_attr("1.0.0"),
        )])));

        // Upgrade package to 1.2.0 and shift local declaration ID (simulating unrelated edits).
        let mut pkg_v2 = base_package(64, "1.2.0");
        let id_v2 = decl(1, 99);
        pkg_v2.structs.push(Struct {
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![decl(1, 100)],
            declaration: id_v2,
            name: "Buffer".into(),
            fields: vec![
                ("len".into(), Term::Primitive(Primitive::Usize)),
                (
                    "data".into(),
                    Term::Pointer(Box::new(Term::Rigid {
                        declaration: decl(1, 100),
                        sort: Sort::Type,
                    })),
                ),
            ],
            packed: PackedMode::None,
        });
        pkg_v2.declaration_paths.insert(
            id_v2,
            CanonicalDeclarationPath {
                package: 1,
                modules: vec!["collections".into()],
                name: "Buffer".into(),
            },
        );
        pkg_v2.definition_attributes = DefinitionAttributes::Frozen(Arc::new(BTreeMap::from([(
            id_v2,
            stable_since_attr("1.0.0"),
        )])));

        let ty_v1 = Term::Nominal {
            declaration: id_v1,
            arguments: vec![Term::Primitive(Primitive::U32)],
        };
        let ty_v2 = Term::Nominal {
            declaration: id_v2,
            arguments: vec![Term::Primitive(Primitive::U32)],
        };
        let mut fuel = 10_000;
        let hash_v1 = compute_type_id(&pkg_v1, &ty_v1, &mut fuel).unwrap();
        let hash_v2 = compute_type_id(&pkg_v2, &ty_v2, &mut fuel).unwrap();
        assert_ne!(hash_v1, 0);
        assert_eq!(hash_v1, hash_v2);

        // Changing field layout under the same stable_since must produce a different hash.
        pkg_v2.structs[0]
            .fields
            .push(("cap".into(), Term::Primitive(Primitive::Usize)));
        let hash_changed = compute_type_id(&pkg_v2, &ty_v2, &mut fuel).unwrap();
        assert_ne!(hash_v1, hash_changed);
    }

    #[test]
    fn recursive_pointer_struct_terminates_and_preserves_topology() {
        let mut pkg = base_package(64, "1.0.0");
        let node_id = decl(1, 1);
        let node_ty = Term::Nominal {
            declaration: node_id,
            arguments: vec![],
        };
        pkg.structs.push(Struct {
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: node_id,
            name: "Node".into(),
            fields: vec![
                ("value".into(), Term::Primitive(Primitive::U32)),
                (
                    "next".into(),
                    Term::Optional(Box::new(Term::Pointer(Box::new(node_ty.clone())))),
                ),
            ],
            packed: PackedMode::None,
        });
        pkg.declaration_paths.insert(
            node_id,
            CanonicalDeclarationPath {
                package: 1,
                modules: vec![],
                name: "Node".into(),
            },
        );
        let mut fuel = 10_000;
        let id64 = compute_type_id(&pkg, &node_ty, &mut fuel).unwrap();
        assert_ne!(id64, 0);
        assert_eq!(id64, compute_type_id(&pkg, &node_ty, &mut fuel).unwrap());

        // 32-bit target pointer size changes ABI layout -> different type_id.
        pkg.target = Some(TargetData::new(32).unwrap());
        let id32 = compute_type_id(&pkg, &node_ty, &mut fuel).unwrap();
        assert_ne!(id64, id32);
    }

    #[test]
    fn rejects_open_generics_missing_target_and_non_global_runtime_origins() {
        let mut pkg = base_package(64, "1.0.0");
        let mut fuel = 10_000;
        let open = Term::Rigid {
            declaration: decl(1, 1),
            sort: Sort::Type,
        };
        assert!(matches!(
            compute_type_id(&pkg, &open, &mut fuel),
            Err(TypeIdError::OpenGeneric(_))
        ));

        let box_id = decl(1, 2);
        pkg.structs.push(Struct {
            parameter_types: vec![Term::Lift(Box::new(Term::Primitive(Primitive::U32)))],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![decl(1, 3)],
            declaration: box_id,
            name: "HeapBox".into(),
            fields: vec![("v".into(), Term::Primitive(Primitive::U32))],
            packed: PackedMode::None,
        });
        let local_rt = Term::Nominal {
            declaration: box_id,
            arguments: vec![Term::Runtime(RuntimeId {
                instance: 0,
                local: 77,
            })],
        };
        assert!(matches!(
            compute_type_id(&pkg, &local_rt, &mut fuel),
            Err(TypeIdError::NonGlobalRuntimeOrigin(RuntimeId {
                instance: 0,
                local: 77
            }))
        ));

        pkg.target = None;
        assert_eq!(
            compute_type_id(&pkg, &Term::Primitive(Primitive::U32), &mut fuel),
            Err(TypeIdError::MissingTarget)
        );
    }

    #[test]
    fn newtype_and_enum_encoding_participate_in_type_id() {
        let mut pkg = base_package(64, "1.0.0");
        let newtype_id = decl(1, 10);
        pkg.newtypes.push(crate::newtypes::Newtype {
            declaration: newtype_id,
            name: "UserId".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements: vec![],
            capabilities: vec![],
            representation: Term::Primitive(Primitive::U64),
        });
        pkg.declaration_paths.insert(
            newtype_id,
            CanonicalDeclarationPath {
                package: 1,
                modules: vec![],
                name: "UserId".into(),
            },
        );
        let enum_id = decl(1, 11);
        pkg.enums.push(Enum {
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: enum_id,
            name: "Color".into(),
            variants: vec![
                Variant {
                    name: "red".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
                Variant {
                    name: "green".into(),
                    shape: VariantShape::Unit,
                    fields: vec![],
                },
            ],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
        });
        pkg.declaration_paths.insert(
            enum_id,
            CanonicalDeclarationPath {
                package: 1,
                modules: vec![],
                name: "Color".into(),
            },
        );
        let mut fuel = 10_000;
        let u64_id = compute_type_id(&pkg, &Term::Primitive(Primitive::U64), &mut fuel).unwrap();
        let nt_id = compute_type_id(
            &pkg,
            &Term::Nominal {
                declaration: newtype_id,
                arguments: vec![],
            },
            &mut fuel,
        )
        .unwrap();
        let enum_a = compute_type_id(
            &pkg,
            &Term::Nominal {
                declaration: enum_id,
                arguments: vec![],
            },
            &mut fuel,
        )
        .unwrap();
        assert_ne!(u64_id, nt_id);
        assert_ne!(enum_a, 0);

        // Renaming an enum variant must change its type_id.
        pkg.enums[0].variants[1].name = "blue".into();
        let enum_b = compute_type_id(
            &pkg,
            &Term::Nominal {
                declaration: enum_id,
                arguments: vec![],
            },
            &mut fuel,
        )
        .unwrap();
        assert_ne!(enum_a, enum_b);

        // Opaque non-Tuple normal form without layout is rejected cleanly.
        let nf_id = decl(1, 12);
        pkg.normal_forms.push(Head {
            collector: None,
            role: crate::normal_forms::Role::Opaque,
            declaration: nf_id,
            parameters: vec![],
            requirements: vec![],
        });
        let nf_ty = Term::NormalForm {
            head: nf_id,
            sort: Sort::Type,
            arguments: vec![],
        };
        assert!(compute_type_id(&pkg, &nf_ty, &mut fuel).is_err());
    }

    #[test]
    fn static_address_comptime_domain_and_error_mappings_are_canonical() {
        // 1. StaticAddress global origin stays invariant when local DeclarationId shifts,
        //    changes when canonical static path changes, and ComptimeFunction domain hashes structurally.
        let make_static_pkg = |static_local: u32, static_name: &str| {
            let mut pkg = base_package(64, "1.0.0");
            let static_id = decl(1, static_local);
            let holder_id = decl(1, 20);
            let origin_id = crate::RuntimeId {
                instance: 1,
                local: 77,
            };
            pkg.declaration_paths.insert(
                static_id,
                CanonicalDeclarationPath {
                    package: 1,
                    modules: vec!["mem".into()],
                    name: static_name.into(),
                },
            );
            pkg.declaration_paths.insert(
                holder_id,
                CanonicalDeclarationPath {
                    package: 1,
                    modules: vec!["mem".into()],
                    name: "Holder".into(),
                },
            );
            pkg.structs.push(Struct {
                parameter_types: vec![Term::Lift(Box::new(Term::Pointer(Box::new(
                    Term::Primitive(Primitive::U64),
                ))))],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![decl(1, 21)],
                declaration: holder_id,
                name: "Holder".into(),
                fields: vec![("tag".into(), Term::Primitive(Primitive::U32))],
                packed: PackedMode::None,
            });
            pkg.runtime_domains.insert(
                origin_id,
                Term::Lift(Box::new(Term::Pointer(Box::new(Term::Primitive(
                    Primitive::U64,
                ))))),
            );
            pkg.global_runtime_deliveries.insert(
                origin_id,
                Expr {
                    ty: Term::Pointer(Box::new(Term::Primitive(Primitive::U64))),
                    source: SourceRange { start: 0, end: 0 },
                    kind: ExprKind::StaticAddress(static_id),
                },
            );
            let ty = Term::Nominal {
                declaration: holder_id,
                arguments: vec![Term::Runtime(origin_id)],
            };
            (pkg, ty, origin_id, holder_id)
        };

        let (pkg_a, ty_a, origin_id, _holder_id) = make_static_pkg(5, "GLOBAL_BUF");
        let (pkg_b, ty_b, _, _) = make_static_pkg(99, "GLOBAL_BUF");
        let (mut pkg_c, ty_c, _, _) = make_static_pkg(99, "OTHER_BUF");
        let mut fuel = 10_000;
        let id_a = compute_type_id(&pkg_a, &ty_a, &mut fuel).unwrap();
        let id_b = compute_type_id(&pkg_b, &ty_b, &mut fuel).unwrap();
        let id_c = compute_type_id(&pkg_c, &ty_c, &mut fuel).unwrap();
        assert_eq!(id_a, id_b);
        assert_ne!(id_a, id_c);

        // 2. Nested Local inside global runtime origin reports the non-zero origin RuntimeId.
        pkg_c.global_runtime_deliveries.insert(
            origin_id,
            Expr {
                ty: Term::Pointer(Box::new(Term::Primitive(Primitive::U64))),
                source: SourceRange { start: 0, end: 0 },
                kind: ExprKind::AddressOf(Box::new(Expr {
                    ty: Term::Primitive(Primitive::U64),
                    source: SourceRange { start: 0, end: 0 },
                    kind: ExprKind::Local(crate::program::LocalId { owner: 0, index: 3 }),
                })),
            },
        );
        assert_eq!(
            compute_type_id(&pkg_c, &ty_c, &mut fuel),
            Err(TypeIdError::NonGlobalRuntimeOrigin(origin_id))
        );

        // 3. Unhandled global expr variant returns UnsupportedGlobalExpr instead of Debug fallback.
        pkg_c.global_runtime_deliveries.insert(
            origin_id,
            Expr {
                ty: Term::Pointer(Box::new(Term::Primitive(Primitive::U64))),
                source: SourceRange { start: 0, end: 0 },
                kind: ExprKind::Return(None),
            },
        );
        assert!(matches!(
            compute_type_id(&pkg_c, &ty_c, &mut fuel),
            Err(TypeIdError::UnsupportedGlobalExpr(_))
        ));

        // 4. ComptimeFunction / NormalFormFunction / UnsafeFunction / Bound domain hashes structurally.
        let world_fn = PackageTypeIdWorld(&pkg_a);
        let mut engine_a = MerkleEngine {
            world: &world_fn,
            fuel: &mut fuel,
            visiting: Vec::new(),
            visiting_runtime: BTreeSet::new(),
            closed_memo: Vec::new(),
        };
        let mut d_fn_a = CanonicalDigest::new(b"test_fn_domain");
        let fn_domain_a = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![crate::types::Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::NormalFormFunction(
                Box::new(Term::UnsafeFunction(Box::new(Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                }))),
                Some(0),
            )),
        }));
        engine_a
            .hash_comptime_domain(&mut d_fn_a, &fn_domain_a)
            .unwrap();
        let mut d_fn_b = CanonicalDigest::new(b"test_fn_domain");
        let fn_domain_b = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![crate::types::Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::NormalFormFunction(
                Box::new(Term::UnsafeFunction(Box::new(Term::Bound {
                    depth: 0,
                    index: 1,
                    sort: Sort::Type,
                }))),
                Some(0),
            )),
        }));
        engine_a
            .hash_comptime_domain(&mut d_fn_b, &fn_domain_b)
            .unwrap();
        assert_ne!(d_fn_a.finish(), d_fn_b.finish());

        // 5. BitPackedError and calling_convention errors map cleanly without collapsing to Unsupported.
        assert_eq!(
            MerkleEngine::<PackageTypeIdWorld<'_>>::map_bit_packed_error(
                crate::BitPackedError::Layout(LayoutError::BudgetExhausted),
            ),
            TypeIdError::BudgetExhausted
        );
        assert_eq!(
            MerkleEngine::<PackageTypeIdWorld<'_>>::map_bit_packed_error(
                crate::BitPackedError::WidthOverflow,
            ),
            TypeIdError::Layout(LayoutError::Overflow)
        );

        let mut pkg_struct = base_package(64, "1.0.0");
        let s_id = decl(1, 30);
        pkg_struct.structs.push(Struct {
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: s_id,
            name: "Pair".into(),
            fields: vec![("x".into(), Term::Primitive(Primitive::U32))],
            packed: PackedMode::None,
        });
        let s_ty = Term::Nominal {
            declaration: s_id,
            arguments: vec![],
        };
        let mut fuel_cc = 10_000;
        pkg_struct.definition_attributes =
            DefinitionAttributes::Pending(Arc::new(BTreeSet::from([s_id])));
        assert_eq!(
            compute_type_id(&pkg_struct, &s_ty, &mut fuel_cc),
            Err(TypeIdError::PendingDefinitionAttributes(s_id))
        );

        pkg_struct.definition_attributes =
            DefinitionAttributes::Frozen(Arc::new(BTreeMap::from([(
                s_id,
                Arc::new(crate::objects::value(vec![crate::objects::entry(
                    Some("__flurry_abi"),
                    Term::Primitive(Primitive::Symbol),
                    Term::Symbol("bogus".into()),
                )])),
            )])));
        assert!(matches!(
            compute_type_id(&pkg_struct, &s_ty, &mut fuel_cc),
            Err(TypeIdError::InvalidAbiMetadata { declaration, .. }) if declaration == s_id
        ));
    }
}
