import Foundation

/// Minimal postcard (serde, no-schema) byte encoder matching the Rust `postcard`
/// crate **byte-for-byte**. One source of truth for the Swift→Rust command channel.
///
/// Wire rules (verified empirically against the engine via golden fixtures):
/// - enum = varint variant index in **Rust declaration order**
/// - struct = fields in declared order
/// - `Vec`/`String`/`[u8]` = varint byte-length prefix + payload
/// - `[T; N]` = N unprefixed elements
/// - `Option<T>` = `0u8` (None) | `1u8` + T (Some)
/// - `u8` = single byte; `u16`/`u32`/`u64`/`usize` = LEB128 varint (usize ≡ u64)
/// - signed ints = zigzag-then-varint
/// - `f32`/`f64` = little-endian IEEE-754 bits
/// - `bool` = one byte
/// - `Uuid` = varint(16) + 16 raw big-endian bytes (uuid crate serde, `serde` feature)
struct PostcardWriter {
    private(set) var bytes: [UInt8] = []

    init() {}

    mutating func writeU8(_ v: UInt8) { bytes.append(v) }
    mutating func writeBool(_ v: Bool) { writeU8(v ? 1 : 0) }

    /// Unsigned LEB128 varint. Backs `u16`/`u32`/`u64`/`usize`.
    mutating func writeVarint(_ v: UInt64) {
        var x = v
        while x >= 0x80 {
            bytes.append(UInt8(x & 0x7F) | 0x80)
            x >>= 7
        }
        bytes.append(UInt8(x))
    }

    /// `usize` (and any unsigned int) — serde models `usize` as `u64`.
    mutating func writeUInt(_ v: UInt) { writeVarint(UInt64(v)) }
    mutating func writeU32(_ v: UInt32) { writeVarint(UInt64(v)) }

    /// Signed int → zigzag → varint.
    mutating func writeI32(_ v: Int32) {
        let zz = (UInt32(bitPattern: v) << 1) ^ UInt32(bitPattern: v >> 31)
        writeVarint(UInt64(zz))
    }

    mutating func writeF32(_ v: Float) {
        withUnsafeBytes(of: v.bitPattern.littleEndian) { bytes.append(contentsOf: $0) }
    }
    mutating func writeF64(_ v: Double) {
        withUnsafeBytes(of: v.bitPattern.littleEndian) { bytes.append(contentsOf: $0) }
    }

    /// `Vec<u8>` / `&[u8]` / `String` payload framing: varint length + raw bytes.
    mutating func writeBytes(_ b: [UInt8]) {
        writeVarint(UInt64(b.count))
        bytes.append(contentsOf: b)
    }
    mutating func writeString(_ s: String) { writeBytes(Array(s.utf8)) }

    /// Enum variant discriminator.
    mutating func writeTag(_ index: UInt) { writeVarint(UInt64(index)) }

    /// `Uuid` = varint(16) + 16 big-endian bytes.
    mutating func writeUUID(_ u: UUID) {
        let t = u.uuid
        writeBytes([t.0, t.1, t.2, t.3, t.4, t.5, t.6, t.7,
                    t.8, t.9, t.10, t.11, t.12, t.13, t.14, t.15])
    }
}
