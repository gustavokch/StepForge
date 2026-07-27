import XCTest
@testable import StepForge

final class HostTransportBuilderTests: XCTestCase {
    func testPlayingTransportAtBarStart() {
        let t = HostTransportBuilder.make(
            sampleRate: 44100, frameCount: 512, tempo: 120.0,
            beat: 0.0, currentDownBeat: 0.0, isPlaying: true)
        XCTAssertEqual(t.tempo_bpm, 120.0)
        XCTAssertEqual(t.sample_rate, 44100.0)
        XCTAssertEqual(t.block_samples, 512)
        XCTAssertEqual(t.block_start_beat, 0.0)
        XCTAssertEqual(t.bar_start_beat, 0.0)
        XCTAssertEqual(t.beats_per_bar, 4.0)     // default; Phase-0 accumulator ignores it
        XCTAssertTrue(t.is_playing)
    }

    func testStoppedTransport() {
        let t = HostTransportBuilder.make(
            sampleRate: 48000, frameCount: 256, tempo: 90.0,
            beat: 7.5, currentDownBeat: 4.0, isPlaying: false)
        XCTAssertFalse(t.is_playing)
        XCTAssertEqual(t.block_start_beat, 7.5)
        XCTAssertEqual(t.bar_start_beat, 4.0)   // passed through for render_host realign
    }

    func testMidBarResumeCarriesDownbeat() {
        // Mid-bar (beat 1.5 within a bar starting at beat 4): render_host aligns
        // step 0 to the downbeat, so the builder must pass bar_start_beat verbatim.
        let t = HostTransportBuilder.make(
            sampleRate: 44100, frameCount: 512, tempo: 140.0,
            beat: 5.5, currentDownBeat: 4.0, isPlaying: true)
        XCTAssertEqual(t.block_start_beat, 5.5)
        XCTAssertEqual(t.bar_start_beat, 4.0)
        XCTAssertEqual(t.tempo_bpm, 140.0)
    }
}
