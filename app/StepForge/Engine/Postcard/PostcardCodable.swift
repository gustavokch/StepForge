import Foundation

/// Conformance seam so `Command`/`EngineEvent`/wire models stay readable instead
/// of inlining raw `PostcardWriter`/`PostcardReader` calls everywhere.
protocol PostcardEncodable {
    func encode(to writer: inout PostcardWriter)
}
protocol PostcardDecodable {
    init?(from reader: inout PostcardReader)
}
typealias PostcardCodable = PostcardEncodable & PostcardDecodable
