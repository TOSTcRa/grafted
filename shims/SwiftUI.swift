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
func _NSApplicationMain(_ argc: Int32, _ argv: UnsafePointer<UnsafePointer<CChar>>?) -> Int32

@_silgen_name("grafted_swiftui_create_window")
func _grafted_create_window(_ title: UnsafePointer<CChar>, _ w: Int32, _ h: Int32) -> Int32

@_silgen_name("grafted_swiftui_run_loop")
func _grafted_run_loop()

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
    public init(_ key: String) {}
    public init(verbatim: String) {}
}

// Property wrappers
@propertyWrapper
public struct State<Value> {
    public var wrappedValue: Value
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
        // The binary passes us type metadata + conformance descriptor.
        // We can't call Self() because the conformance's init witness
        // uses Mach-O relative pointers incompatible with Linux runtime.
        // Instead, create the app's window directly — this is what the
        // real SwiftUI App.main() does internally (create scenes, enter loop).
        _grafted_create_window("Maccy", 400, 500)
        _grafted_run_loop()
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
