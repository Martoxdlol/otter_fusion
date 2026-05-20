use crate::{
    hir::ResolvedType,
    lower::{Lower, builder::FnBuilder, mangling::name_type, subst::Subst},
    mir::*,
};

impl Lower {
    /// Get or create a tagged-union MirTypeId for these variants.
    ///
    /// Variants are sorted lexicographically by their stringified name so
    /// `A | B` and `B | A` agree on tags. Caller must have already ruled
    /// out the `T | null` (managed) specialisation via `try_nullable_ref`.
    pub fn get_or_create_union(&mut self, variants: Vec<ResolvedType>) -> MirTypeId {
        let sorted = self.sort_union_variants(variants);
        if let Some(&mid) = self.mono_unions.get(&sorted) {
            return mid;
        }
        let mid = self.alloc_type_id();
        self.mono_unions.insert(sorted.clone(), mid);

        // Placeholder for recursive references.
        self.mir.types.insert(
            mid,
            MirTypeDef::Union {
                variants: vec![],
                layout: Layout { size: UNION_TOTAL_SIZE, align: UNION_ALIGN },
            },
        );

        let union_variants = self.build_union_variants(&sorted);
        self.mir.types.insert(
            mid,
            MirTypeDef::Union {
                variants: union_variants,
                layout: Layout { size: UNION_TOTAL_SIZE, align: UNION_ALIGN },
            },
        );
        mid
    }

    /// `T | null` where T lowers to ManagedRef(mid) → Some(mid). Caller
    /// uses this to short-circuit to NullableRef instead of a tagged union.
    pub fn try_nullable_ref(&mut self, variants: &[ResolvedType]) -> Option<MirTypeId> {
        if variants.len() != 2 {
            return None;
        }
        let has_null = variants.iter().any(|t| matches!(t, ResolvedType::Null));
        if !has_null {
            return None;
        }
        let other = variants
            .iter()
            .find(|t| !matches!(t, ResolvedType::Null))
            .cloned()?;
        match self.lower_concrete_type(&other) {
            MirType::ManagedRef(mid) => Some(mid),
            _ => None,
        }
    }

    fn sort_union_variants(&self, mut variants: Vec<ResolvedType>) -> Vec<ResolvedType> {
        variants.sort_by_key(|t| name_type(&self.hir, t, &[]));
        variants
    }

    fn build_union_variants(&mut self, sorted: &[ResolvedType]) -> Vec<UnionVariant> {
        let mut next_tag: u16 = 1;
        let mut out = Vec::with_capacity(sorted.len());
        for v in sorted {
            let tag = if matches!(v, ResolvedType::Null) {
                NULL_TAG
            } else {
                let t = next_tag;
                next_tag += 1;
                t
            };
            let ty = self.lower_concrete_type(v);
            out.push(UnionVariant { tag, ty });
        }
        out
    }

    /// Tag that `variant` would get inside a union with these variants.
    /// Variants are taken pre-sort; sorting is applied internally.
    fn union_tag_for(&self, variants: &[ResolvedType], variant: &ResolvedType) -> u16 {
        if matches!(variant, ResolvedType::Null) {
            return NULL_TAG;
        }
        let mut sorted: Vec<ResolvedType> = variants.to_vec();
        sorted.sort_by_key(|t| name_type(&self.hir, t, &[]));
        let mut next_tag: u16 = 1;
        for v in &sorted {
            if matches!(v, ResolvedType::Null) {
                continue;
            }
            if v == variant {
                return next_tag;
            }
            next_tag += 1;
        }
        panic!("variant {:?} not present in union {:?}", variant, variants);
    }

    /// Widen `op` (of type `from`) into the target type `to`, emitting a
    /// UnionConstruct if the target is a tagged union. No-op when types
    /// already match, or when the target is a `NullableRef` (same repr).
    /// Both types should be concrete (TypeParams resolved).
    pub fn coerce_to(
        &mut self,
        op: Operand,
        from: &ResolvedType,
        to: &ResolvedType,
        b: &mut FnBuilder,
    ) -> Operand {
        if from == to {
            return op;
        }
        match to {
            ResolvedType::Union(target_vs) => self.widen_to_union(op, from, target_vs, b),
            _ => op,
        }
    }

    /// Apply caller substitution to both sides before coercing. Convenience
    /// wrapper for call sites where types are still in the caller scope.
    pub fn coerce_to_subst(
        &mut self,
        op: Operand,
        from: &ResolvedType,
        to: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let from = subst.apply(from);
        let to = subst.apply(to);
        self.coerce_to(op, &from, &to, b)
    }

    fn widen_to_union(
        &mut self,
        op: Operand,
        from: &ResolvedType,
        target_vs: &[ResolvedType],
        b: &mut FnBuilder,
    ) -> Operand {
        let target_mir = self.lower_concrete_type(&ResolvedType::Union(target_vs.to_vec()));
        match target_mir {
            // NullableRef and the underlying T share representation, and
            // `null` lowers to pointer-0, also a valid NullableRef.
            MirType::NullableRef(_) => op,
            MirType::Union(uid) => {
                let tag = self.union_tag_for(target_vs, from);
                b.emit(
                    AssignValue::UnionConstruct(uid, tag as u32, op),
                    MirType::Union(uid),
                )
            }
            _ => unreachable!("union target lowered to {:?}", target_mir),
        }
    }
}
