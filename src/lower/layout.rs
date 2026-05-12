use crate::hir::PrimitiveType;
use crate::mir::*;

// Tamaño y alineación de cada tipo primitivo, en bytes.
// El alineamiento siempre es potencia de 2 (lo asume align_up).
// String se trata como puntero porque en runtime es una referencia gestionada.
pub fn primitive_size_align(p: &PrimitiveType) -> (u32, u32) {
    use PrimitiveType::*;
    match p {
        Bool | Int8 | Uint8 => (1, 1),
        Int16 | Uint16 => (2, 2),
        Int32 | Uint32 | Float32 | Char => (4, 4),
        Int64 | Uint64 | Float64 => (8, 8),
        String => (POINTER_SIZE, POINTER_ALIGN),
    }
}

// Tamaño y alineación de un MirType cuando vive inline dentro de un struct.
// Todo lo que es por referencia (ManagedRef, Pointer, NullableRef, FnPtr, Closure)
// ocupa el espacio de un puntero. Las uniones ya traen su layout precomputado
// en el MirTypeDef, así que lo leemos directo de ahí.
pub fn type_size_align(prog: &MirProgram, ty: &MirType) -> (u32, u32) {
    match ty {
        MirType::Primitive(p) => primitive_size_align(p),
        MirType::ManagedRef(_) => (POINTER_SIZE, POINTER_ALIGN),
        MirType::Pointer(_) => (POINTER_SIZE, POINTER_ALIGN),
        MirType::NullableRef(_) => (POINTER_SIZE, POINTER_ALIGN),
        MirType::FnPtr(_, _) => (POINTER_SIZE, POINTER_ALIGN),
        MirType::Closure(_) => (POINTER_SIZE, POINTER_ALIGN),
        MirType::Union(uid) => match &prog.types[uid] {
            MirTypeDef::Union { layout, .. } => (layout.size, layout.align),
            _ => unreachable!("MirType::Union pointing at non-union"),
        },
    }
}

// Redondea offset hacia arriba al múltiplo de align más cercano.
// Truco clásico de bit-twiddling: solo funciona si align es potencia de 2,
// por eso el debug_assert. (offset + align - 1) sube al siguiente múltiplo
// y la máscara !(align - 1) descarta los bits bajos.
fn align_up(offset: u32, align: u32) -> u32 {
    debug_assert!(align.is_power_of_two(), "align must be a power of two");
    (offset + align - 1) & !(align - 1)
}

// Calcula los offsets de cada campo y el layout total de un struct.
//
// Algoritmo (igual al que usan C, Rust, etc.):
//   1. Arrancamos en offset 0.
//   2. Para cada campo, alineamos el offset actual al alineamiento del campo,
//      anotamos ese offset como la posición del campo, y avanzamos por su tamaño.
//   3. El alineamiento del struct entero es el máximo de los alineamientos de
//      sus campos.
//   4. El tamaño total se redondea al alineamiento del struct, para que un
//      array de structs siga manteniendo los campos alineados.
//
// No reordenamos campos: respetamos el orden de declaración, por lo que el
// usuario puede tener padding entre medio si pone campos chicos antes de
// grandes. Es predecible aunque no sea óptimo.
pub fn compute_struct_layout(prog: &MirProgram, field_types: &[MirType]) -> (Vec<u32>, Layout) {
    let mut offset = 0u32;
    let mut align = 1u32;
    let mut offsets = Vec::with_capacity(field_types.len());
    for ty in field_types {
        let (sz, al) = type_size_align(prog, ty);
        offset = align_up(offset, al);
        offsets.push(offset);
        offset += sz;
        if al > align {
            align = al;
        }
    }
    let size = align_up(offset, align);
    (offsets, Layout { size, align })
}
