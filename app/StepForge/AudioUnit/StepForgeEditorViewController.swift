#if os(macOS)
import AudioToolbox
import CoreAudioKit
import AppKit
import SwiftUI

/// The AU extension's principal class (NSExtensionPrincipalClass): vends the
/// `AUAudioUnit` and hosts the editor. Phase 1 hosts a placeholder; Task 6
/// swaps in `PluginEditorView`.
final class StepForgeEditorViewController: AUViewController, AUAudioUnitFactory {

    private var audioUnit: StepForgeAudioUnit?

    func createAudioUnit(with componentDescription: AudioComponentDescription) throws -> AUAudioUnit {
        let au = try StepForgeAudioUnit(componentDescription: componentDescription)
        self.audioUnit = au
        return au
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // Placeholder view until Task 6 binds the editor.
        view = NSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.frame = NSRect(x: 0, y: 0, width: 760, height: 520)
    }
}
#endif
