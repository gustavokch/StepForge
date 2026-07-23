import Foundation

/// Symmetric postcard decoder (see `PostcardWriter` for the wire rules). Every
/// read returns nil on truncation; callers propagate nil up so a malformed event
/// is **dropped**, never crashes the app (CLAUDE.md Hard Rule 3: no panic across FFI).
struct PostcardReader {
    private let bytes: [UInt8]
    private var pos: Int

    init(_ bytes: [UInt8]) { self.bytes = bytes; self.pos = 0; }

    mutating func readU8() -> UInt8? {
        guard pos < bytes.count else { return nil; }
        let b = bytes[pos]; pos += 1; return b;
    }
    mutating func readBool() -> Bool? { guard let b = readU8() else { return nil; }; return b != 0; }

    /// Unsigned LEB128 varint (≤10 bytes, the max for a u64). Nil if unterminated.
    mutating func readVarint() -> UInt64? {
        var result: UInt64 = 0
        var shift: UInt64 = 0
        for _ in 0..<10 {
            guard pos < bytes.count else { return nil; }
            let b = bytes[pos]; pos += 1
            result |= UInt64(b & 0x7F) << shift
            if b < 0x80 { return result; }
            shift += 7
        }
        return nil
    }

    mutating func readUInt() -> Int? { guard let v = readVarint() else { return nil; }; return Int(truncatingIfNeeded: v); }
    mutating func readU32() -> UInt32? { guard let v = readVarint() else { return nil; }; return UInt32(truncatingIfNeeded: v); }

    /// zigzag-decoded signed i32.
    mutating func readI32() -> Int32? {
        guard let zz = readVarint() else { return nil; }
        let u = UInt32(truncatingIfNeeded: zz)
        let unzig = (u >> 1) ^ (0 &- (u & 1))   // (u >>> 1) ^ -(u & 1)
        return Int32(bitPattern: unzig)
    }

    mutating func readF32() -> Float? {
        guard pos + 4 <= bytes.count else { return nil; }
        let raw = bytes.withUnsafeBufferPointer { buf -> UInt32 in
            var v: UInt32 = 0
            for i in 0..<4 { v |= UInt32(buf[pos + i]) << (8 * i); }
            return v
        }
        pos += 4
        return Float(bitPattern: raw)
    }
    mutating func readF64() -> Double? {
        guard pos + 8 <= bytes.count else { return nil; }
        let raw = bytes.withUnsafeBufferPointer { buf -> UInt64 in
            var v: UInt64 = 0
            for i in 0..<8 { v |= UInt64(buf[pos + i]) << (8 * i); }
            return v
        }
        pos += 8
        return Double(bitPattern: raw)
    }

    /// Length-prefixed byte slice (`Vec<u8>` / `String` payload / `Uuid`).
    mutating func readBytes() -> [UInt8]? {
        guard let len = readVarint() else { return nil; }
        let n = Int(truncatingIfNeeded: len)
        guard pos + n <= bytes.count else { return nil; }
        let out = Array(bytes[pos..<(pos + n)]); pos += n; return out
    }
    mutating func readString() -> String? {
        guard let b = readBytes() else { return nil; }
        return String(bytes: b, encoding: .utf8)
    }
    mutating func readTag() -> Int? { readUInt(); }

    /// `Uuid` = varint(16) + 16 big-endian bytes.
    mutating func readUUID() -> UUID? {
        guard let b = readBytes(), b.count == 16 else { return nil; }
        return UUID(uuid: (b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                           b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]))
    }

    /// `Option<T>`: returns nil on truncation (error), `.some(nil)` for None,
    /// `.some(value)` for Some — i.e. an `Optional<Optional<T>>` (`T??`).
    mutating func readOption<T>(_ decode: (inout PostcardReader) -> T?) -> T?? {
        guard let tag = readU8() else { return nil; }      // truncated
        if tag == 0 { return .some(nil); }                 // None
        guard let value = decode(&self) else { return nil; } // Some, but payload truncated
        return .some(value)
    }
}
