#if os(macOS)
import AudioToolbox
import CoreAudioKit
import AppKit
import SwiftUI

/// The AU extension's principal class (NSExtensionPrincipalClass): vends the
/// `AUAudioUnit` and hosts the SwiftUI editor. `viewDidLoad` binds the borrowed
/// `EngineBridge` (the AU owns the host-driven handle) into a `PluginEditorView`
/// via `NSHostingView`; the drain timer (~120 Hz) refreshes the mirror the
/// editor reads, and gestures submit commands through the same bridge.
final class StepForgeEditorViewController: AUViewController, AUAudioUnitFactory {

    private var audioUnit: StepForgeAudioUnit?

    func createAudioUnit(with componentDescription: AudioComponentDescription) throws -> AUAudioUnit {
        let au = try StepForgeAudioUnit(componentDescription: componentDescription)
        self.audioUnit = au
        return au
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // The AU owns the borrowed bridge (host-driven handle); the editor binds
        // to it via `\.environmentObject`. The drain timer refreshes the mirror
        // that SwiftUI reads (~120 Hz); gestures submit commands through it.
        guard let au = audioUnit else { return }
        let bridge = au.bridgeForEditor()
        view = NSHostingView(rootView:
            PluginEditorView()
                .environmentObject(bridge)
                .environment(\.usePluginTransport, true))
        view.frame = NSRect(x: 0, y: 0, width: 760, height: 520)
        // Seed the mirror with a full snapshot so the editor isn't blank for one
        // drain tick after open. No-op if the snapshot is already in flight.
        bridge.requestSnapshot()
    }
}
#endif
