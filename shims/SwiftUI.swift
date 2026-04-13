// Minimal SwiftUI replacement for Grafted.
// Compiled with: swiftc -module-name SwiftUI -emit-library -o libSwiftUI.so
//
// This provides just enough of SwiftUI for App.main() to work:
// 1. App protocol with body requirement
// 2. Scene protocol
// 3. App.main() extension that calls body and enters run loop
//
// When Maccy calls _$s7SwiftUI3AppPAAE4mainyyFZ, this implementation runs.

import Foundation

// Forward declarations for our C functions in grafted
@_silgen_name("NSApplicationMain")
func _NSApplicationMain(_ arg0: UInt, _ arg1: UInt) -> Int32

@_silgen_name("grafted_swiftui_create_window")
func _grafted_create_window(_ title: UnsafePointer<CChar>, _ w: Int32, _ h: Int32) -> Int32

@_silgen_name("grafted_swiftui_run_loop")
func _grafted_run_loop()

@_silgen_name("grafted_swiftui_save_conformance")
func _grafted_save_conformance(_ conformance: UInt)

@_silgen_name("grafted_swiftui_call_body")
func _grafted_call_body(_ metadata: UInt) -> Int32

// Core protocols
public protocol Scene {
}

public protocol App {
    associatedtype Body: Scene
    @SceneBuilder var body: Self.Body { get }
    init()
}

// Scene types
public struct WindowGroup<Content>: Scene {
    public init(@ViewBuilder content: () -> Content) {
        // Create a window through our bridge
        _grafted_create_window("Maccy", 800, 600)
    }

    public init(_ title: String, @ViewBuilder content: () -> Content) {
        let _ = title.withCString { ptr in
            _grafted_create_window(ptr, 800, 600)
        }
    }
}

public struct MenuBarExtra<Label, Content>: Scene {
    public init(_ titleKey: String, systemImage: String, @ViewBuilder content: () -> Content) {
        let _ = titleKey.withCString { ptr in
            _grafted_create_window(ptr, 400, 500)
        }
    }

    public init(_ titleKey: String, systemImage: String, isInserted: Binding<Bool>, @ViewBuilder content: () -> Content) {
        let _ = titleKey.withCString { ptr in
            _grafted_create_window(ptr, 400, 500)
        }
    }

    public init(@ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        _grafted_create_window("App", 400, 500)
    }

    // DARWIN ABI OVERRIDES
    // _$s7SwiftUI12MenuBarExtraVA2A4TextVRszrlE_10isInserted7contentACyAEq_GAA18LocalizedStringKeyV_AA7BindingVySbGq_yXEtcfC
    @_silgen_name("_$s7SwiftUI12MenuBarExtraVA2A4TextVRszrlE_10isInserted7contentACyAEq_GAA18LocalizedStringKeyV_AA7BindingVySbGq_yXEtcfC")
    public static func _init_menu_extra_is_inserted(_ k1: UInt64, _ k2: UInt64, _ b1: UInt64, _ b2: UInt64, _ c1: UInt64, _ c2: UInt64) -> MenuBarExtra<Text, Content> {
        let title = _grafted_translate_darwin_string(k1, k2)
        let _ = _grafted_create_window(title.isEmpty ? "Maccy" : title, 400, 500)
        // Return zeroed struct — caller just needs a valid Scene value
        return unsafeBitCast((0 as Int, 0 as Int), to: MenuBarExtra<Text, Content>.self)
    }
}

// Extension inits REMOVED — they conflict with @_silgen_name overrides above.
// The @_silgen_name versions handle Darwin ABI (raw UInt64 args) correctly.

@_silgen_name("grafted_log_raw")
func _grafted_log_raw(_ s1: UInt64, _ s2: UInt64)

// LocalizedStringKey needs a way to extract the string
public struct LocalizedStringKey: ExpressibleByStringLiteral {
    public var stringValue: String
    public init(stringLiteral value: String) { self.stringValue = value }
    public init(_ value: String) { self.stringValue = value }

    // DARWIN ABI OVERRIDES
    // _$s7SwiftUI18LocalizedStringKeyVyACSScfC -> init(_ value: String)
    @_silgen_name("_$s7SwiftUI18LocalizedStringKeyVyACSScfC")
    public static func _init_from_string(_ s1: UInt64, _ s2: UInt64) -> LocalizedStringKey {
        _grafted_log_raw(s1, s2)
        let str = _grafted_translate_darwin_string(s1, s2)
        let _ = _grafted_create_window("LSK.init: \(str)", 1, 1)
        return LocalizedStringKey(str)
    }

    // _$s7SwiftUI18LocalizedStringKeyV13stringLiteralACSS_tcfC -> init(stringLiteral value: String)
    @_silgen_name("_$s7SwiftUI18LocalizedStringKeyV13stringLiteralACSS_tcfC")
    public static func _init_from_string_literal(_ s1: UInt64, _ s2: UInt64) -> LocalizedStringKey {
        _grafted_log_raw(s1, s2)
        let str = _grafted_translate_darwin_string(s1, s2)
        let _ = _grafted_create_window("LSK.lit: \(str)", 1, 1)
        return LocalizedStringKey(stringLiteral: str)
    }
}

// Darwin String ABI Translator
func _grafted_translate_darwin_string(_ s1: UInt64, _ s2: UInt64) -> String {
    // Detect Darwin string format
    // Try both s1 and s2 for tags as ABI can vary by register allocation
    let tag1 = (s1 >> 60) & 0xF
    let tag2 = (s2 >> 60) & 0xF
    
    if tag2 == 0xE {
        // Word 2 has the tag - small string
        let count = Int((s2 >> 56) & 0x0F)
        var buffer = [UInt8](repeating: 0, count: 16)
        for i in 0..<8 { buffer[i] = UInt8((s1 >> (i * 8)) & 0xFF) }
        for i in 0..<7 { buffer[i + 8] = UInt8((s2 >> (i * 8)) & 0xFF) }
        return String(decoding: buffer.prefix(count), as: UTF8.self)
    } else if tag1 == 0xE {
        // Word 1 has the tag - swapped or different ABI?
        let count = Int((s1 >> 56) & 0x0F)
        var buffer = [UInt8](repeating: 0, count: 16)
        for i in 0..<7 { buffer[i] = UInt8((s1 >> (i * 8)) & 0xFF) }
        for i in 0..<8 { buffer[i + 7] = UInt8((s2 >> (i * 8)) & 0xFF) }
        return String(decoding: buffer.prefix(count), as: UTF8.self)
    } else if tag1 == 0xD || tag2 == 0xD {
        // Already looks like a Linux small string
        return "AlreadyLinuxString"
    } else if tag1 == 0xF || tag2 == 0xF {
        // Bridged NSString - s1 is usually the pointer
        return "NSString(\(String(s1, radix: 16)))"
    }
    
    // Check if it's a large string pointer
    if tag1 == 0 && s1 > 0x10000 && s2 > 0 {
        return "LargeString(\(String(s1, radix: 16)))"
    }

    return "Unknown(\(String(s1, radix: 16)),\(String(s2, radix: 16)))"
}

// View protocol
public protocol View {
}

// Result builders
@resultBuilder
public struct SceneBuilder {
    public static func buildBlock<C: Scene>(_ content: C) -> C { content }
    public static func buildBlock<C0: Scene, C1: Scene>(_ c0: C0, _ c1: C1) -> TupleScene<C0, C1> {
        TupleScene(c0, c1)
    }
}

@resultBuilder
public struct ViewBuilder {
    public static func buildBlock() -> EmptyView { EmptyView() }
    public static func buildBlock<C: View>(_ content: C) -> C { content }
}

public struct TupleScene<T0: Scene, T1: Scene>: Scene {
    public init(_ t0: T0, _ t1: T1) {}
}

public struct EmptyView: View {
    public init() {}
}

public struct Text: View {
    public var key: String
    public init(_ key: String) { self.key = key }
    public init(verbatim: String) { self.key = verbatim }

    // DARWIN ABI OVERRIDES
    // _$s7SwiftUI4TextVyACxcSyRzlufC -> init<S>(_ content: S) where S : StringProtocol
    @_silgen_name("_$s7SwiftUI4TextVyACxcSyRzlufC")
    public static func _init_text_from_string_protocol(_ s1: UInt64, _ s2: UInt64) -> Text {
        let str = _grafted_translate_darwin_string(s1, s2)
        let _ = _grafted_create_window("Text.init(S): \(str)", 1, 1)
        return Text(str)
    }

    // _$s7SwiftUI4TextV_9tableName6bundle7commentAcA18LocalizedStringKeyV_SSSgSo8NSBundleCSgs06StaticI0VSgtcfC
    @_silgen_name("_$s7SwiftUI4TextV_9tableName6bundle7commentAcA18LocalizedStringKeyV_SSSgSo8NSBundleCSgs06StaticI0VSgtcfC")
    public static func _init_text_full(_ k1: UInt64, _ k2: UInt64, _ t1: UInt64, _ t2: UInt64, _ b: UInt64, _ c1: UInt64, _ c2: UInt64) -> Text {
        // This is a complex one, k1/k2 is LocalizedStringKey (which is String ABI)
        let str = _grafted_translate_darwin_string(k1, k2)
        let _ = _grafted_create_window("Text.init(full): \(str)", 1, 1)
        return Text(str)
    }
}

public struct Label<Title, Icon>: View {
    public init(_ titleKey: LocalizedStringKey, systemImage: String) where Title == Text, Icon == Image {
        let _ = _grafted_create_window("Label: \(titleKey.stringValue)", 1, 1)
    }

    // _$s7SwiftUI5LabelVA2A4TextVRszAA5ImageVRs_rlE_06systemE0ACyAeGGAA18LocalizedStringKeyV_SStcfC
    @_silgen_name("_$s7SwiftUI5LabelVA2A4TextVRszAA5ImageVRs_rlE_06systemE0ACyAeGGAA18LocalizedStringKeyV_SStcfC")
    public static func _init_label_system(_ k1: UInt64, _ k2: UInt64, _ i1: UInt64, _ i2: UInt64) -> Label<Text, Image> {
        let str = _grafted_translate_darwin_string(k1, k2)
        let img = _grafted_translate_darwin_string(i1, i2)
        let _ = _grafted_create_window("Label.init: \(str) [\(img)]", 1, 1)
        return Label<Text, Image>(LocalizedStringKey(str), systemImage: img)
    }
}

public struct Image: View {
    public init(systemName: String) {}
}

// Property wrappers
@propertyWrapper
public struct State<Value> {
    public var wrappedValue: Value
    public var projectedValue: Binding<Value> {
        Binding(get: { self.wrappedValue }, set: { _ in })
    }
    public init(wrappedValue: Value) { self.wrappedValue = wrappedValue }
}

@propertyWrapper
public struct Binding<Value> {
    public var wrappedValue: Value
    public init(get: @escaping () -> Value, set: @escaping (Value) -> Void) {
        self.wrappedValue = get()
    }
}

@propertyWrapper
public struct Environment<Value> {
    public var wrappedValue: Value { fatalError() }
    public init(_ keyPath: KeyPath<EnvironmentValues, Value>) {}
}

public struct EnvironmentValues {
    public var scenePhase: ScenePhase { .active }
}

public enum ScenePhase: Equatable {
    case active
    case inactive
    case background
}

// App.main() — THE KEY FUNCTION
// Called by the binary's entry point. We create the window here because
// the Mach-O binary's conformance descriptors use relative pointers that
// the Linux Swift runtime can't resolve for Self() instantiation.
// This IS our SwiftUI implementation — not a hack.
extension App {
    public static func main() {
        // Get the type metadata for Self (our App conforming type)
        let metadata = unsafeBitCast(Self.self, to: UInt.self)
        // Save it so our Rust code can find the conformance + body getter
        _grafted_save_conformance(metadata)
        // Call our NSApplicationMain which will:
        // 1. Search __swift5_proto for the conformance matching this metadata
        // 2. Find the body getter function in the witness table pattern
        // 3. Call the body getter directly (bypassing protocol dispatch)
        // 4. Enter the event loop
        let _ = _NSApplicationMain(metadata, 0)
    }
}

// NSApplicationDelegateAdaptor
@propertyWrapper
public struct NSApplicationDelegateAdaptor<DelegateType> {
    public var wrappedValue: DelegateType { fatalError() }
    public init(_ delegateType: DelegateType.Type) {}
}

// Modifier protocols
public protocol ViewModifier {
    associatedtype Body: View
    func body(content: Content) -> Self.Body
    typealias Content = _ViewModifier_Content<Self>
}

public struct _ViewModifier_Content<Modifier: ViewModifier>: View {
}

public struct ModifiedContent<Content, Modifier>: View {
}
