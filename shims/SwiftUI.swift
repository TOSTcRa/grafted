// Minimal SwiftUI replacement for Grafted.
// Compiled with: swiftc -module-name SwiftUI -emit-library -emit-module -o shims/libSwiftUI.so shims/SwiftUI.swift -L swift-runtime/usr/lib/swift/linux -Xclang-linker -fuse-ld=lld
//
// This provides just enough of SwiftUI for App.main() to work:
// 1. App protocol with body requirement
// 2. Scene protocol + MenuBarExtra/WindowGroup
// 3. View types (Text, Button, List, etc.) that log to the view tree
// 4. App.main() extension that calls body and enters run loop
//
// When Maccy calls _$s7SwiftUI3AppPAAE4mainyyFZ, this implementation runs.

import Foundation

// ── C bridge declarations ──────────────────────────────────────────────

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

@_silgen_name("grafted_swiftui_log_view")
func _grafted_log_view(_ type: UnsafePointer<CChar>, _ detail: UnsafePointer<CChar>)

@_silgen_name("grafted_call_content_closure")
func _grafted_call_closure(_ fn: UInt64, _ ctx: UInt64) -> Int32

@_silgen_name("grafted_log_raw")
func _grafted_log_raw(_ s1: UInt64, _ s2: UInt64)

// Helper to log a view creation
func _logView(_ type: String, _ detail: String = "") {
    type.withCString { t in
        detail.withCString { d in
            _grafted_log_view(t, d)
        }
    }
}

// ── Core protocols ─────────────────────────────────────────────────────

public protocol Scene {}

public protocol App {
    associatedtype Body: Scene
    @SceneBuilder var body: Self.Body { get }
    init()
}

// ── Scene types ────────────────────────────────────────────────────────

public struct WindowGroup<Content>: Scene {
    public init(@ViewBuilder content: () -> Content) {
        _grafted_create_window("App", 800, 600)
    }
    public init(_ title: String, @ViewBuilder content: () -> Content) {
        let _ = title.withCString { ptr in _grafted_create_window(ptr, 800, 600) }
    }
}

public struct MenuBarExtra<Label, Content>: Scene {
    public init(_ titleKey: String, systemImage: String, @ViewBuilder content: () -> Content) {
        let _ = titleKey.withCString { ptr in _grafted_create_window(ptr, 400, 500) }
    }
    public init(_ titleKey: String, systemImage: String, isInserted: Binding<Bool>, @ViewBuilder content: () -> Content) {
        let _ = titleKey.withCString { ptr in _grafted_create_window(ptr, 400, 500) }
    }
    public init(@ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        _grafted_create_window("App", 400, 500)
    }

    // DARWIN ABI OVERRIDE — the actual symbol the binary calls
    // ABI layout (from disassembly of Maccy body getter):
    //   rdi,rsi = LocalizedStringKey (Darwin small string)
    //   rdx,rcx,r8,r9 = Binding<Bool> parts
    //   stack[0] = extra binding data
    //   stack[1] = content closure fn ptr (in Maccy binary range)
    //   stack[2] = content closure context (0 = non-capturing)
    //   stack[3],stack[4] = Content type metadata + witness table
    @_silgen_name("_$s7SwiftUI12MenuBarExtraVA2A4TextVRszrlE_10isInserted7contentACyAEq_GAA18LocalizedStringKeyV_AA7BindingVySbGq_yXEtcfC")
    public static func _init_menu_extra_is_inserted(
        _ k1: UInt64, _ k2: UInt64,
        _ b1: UInt64, _ b2: UInt64,
        _ b3: UInt64, _ b4: UInt64,
        _ b5: UInt64,
        _ closureFn: UInt64, _ closureCtx: UInt64,
        _ contentMeta: UInt64, _ contentWT: UInt64
    ) -> MenuBarExtra<Text, Content> {
        let title = _grafted_translate_darwin_string(k1, k2)
        let _ = _grafted_create_window(title.isEmpty ? "Maccy" : title, 400, 500)

        // Call the content closure if it's a real function (not a nop)
        if closureFn >= 0x100000000 && closureFn < 0x100200000 {
            let _ = _grafted_call_closure(closureFn, closureCtx)
        }

        // Enter event loop — the real Maccy UI comes from AppDelegate,
        // not from this content closure (which returns EmptyView).
        _grafted_run_loop()
        fatalError("unreachable")
    }
}

// ── Darwin String ABI Translator ───────────────────────────────────────

func _grafted_translate_darwin_string(_ s1: UInt64, _ s2: UInt64) -> String {
    let tag1 = (s1 >> 60) & 0xF
    let tag2 = (s2 >> 60) & 0xF

    if tag2 == 0xE {
        let count = Int((s2 >> 56) & 0x0F)
        var buffer = [UInt8](repeating: 0, count: 16)
        for i in 0..<8 { buffer[i] = UInt8((s1 >> (i * 8)) & 0xFF) }
        for i in 0..<7 { buffer[i + 8] = UInt8((s2 >> (i * 8)) & 0xFF) }
        return String(decoding: buffer.prefix(count), as: UTF8.self)
    } else if tag1 == 0xE {
        let count = Int((s1 >> 56) & 0x0F)
        var buffer = [UInt8](repeating: 0, count: 16)
        for i in 0..<7 { buffer[i] = UInt8((s1 >> (i * 8)) & 0xFF) }
        for i in 0..<8 { buffer[i + 7] = UInt8((s2 >> (i * 8)) & 0xFF) }
        return String(decoding: buffer.prefix(count), as: UTF8.self)
    } else if tag1 == 0xD || tag2 == 0xD {
        return "AlreadyLinuxString"
    } else if tag1 == 0xF || tag2 == 0xF {
        return "NSString(\(String(s1, radix: 16)))"
    }

    if tag1 == 0 && s1 > 0x10000 && s2 > 0 {
        return "LargeString(\(String(s1, radix: 16)))"
    }

    return "Unknown(\(String(s1, radix: 16)),\(String(s2, radix: 16)))"
}

// ── LocalizedStringKey ─────────────────────────────────────────────────

public struct LocalizedStringKey: ExpressibleByStringLiteral {
    public var stringValue: String
    public init(stringLiteral value: String) { self.stringValue = value }
    public init(_ value: String) { self.stringValue = value }

    @_silgen_name("_$s7SwiftUI18LocalizedStringKeyVyACSScfC")
    public static func _init_from_string(_ s1: UInt64, _ s2: UInt64) -> LocalizedStringKey {
        _grafted_log_raw(s1, s2)
        let str = _grafted_translate_darwin_string(s1, s2)
        return LocalizedStringKey(str)
    }

    @_silgen_name("_$s7SwiftUI18LocalizedStringKeyV13stringLiteralACSS_tcfC")
    public static func _init_from_string_literal(_ s1: UInt64, _ s2: UInt64) -> LocalizedStringKey {
        _grafted_log_raw(s1, s2)
        let str = _grafted_translate_darwin_string(s1, s2)
        return LocalizedStringKey(stringLiteral: str)
    }
}

// ── View protocol ──────────────────────────────────────────────────────

public protocol View {}

// ── Result builders ────────────────────────────────────────────────────

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
    public static func buildBlock<C0: View, C1: View>(_ c0: C0, _ c1: C1) -> TupleView<(C0, C1)> {
        TupleView((c0, c1))
    }
    public static func buildBlock<C0: View, C1: View, C2: View>(_ c0: C0, _ c1: C1, _ c2: C2) -> TupleView<(C0, C1, C2)> {
        TupleView((c0, c1, c2))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View>(_ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3) -> TupleView<(C0, C1, C2, C3)> {
        TupleView((c0, c1, c2, c3))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4
    ) -> TupleView<(C0, C1, C2, C3, C4)> {
        TupleView((c0, c1, c2, c3, c4))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View, C5: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4, _ c5: C5
    ) -> TupleView<(C0, C1, C2, C3, C4, C5)> {
        TupleView((c0, c1, c2, c3, c4, c5))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View, C5: View, C6: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4, _ c5: C5, _ c6: C6
    ) -> TupleView<(C0, C1, C2, C3, C4, C5, C6)> {
        TupleView((c0, c1, c2, c3, c4, c5, c6))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View, C5: View, C6: View, C7: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4, _ c5: C5, _ c6: C6, _ c7: C7
    ) -> TupleView<(C0, C1, C2, C3, C4, C5, C6, C7)> {
        TupleView((c0, c1, c2, c3, c4, c5, c6, c7))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View, C5: View, C6: View, C7: View, C8: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4, _ c5: C5, _ c6: C6, _ c7: C7, _ c8: C8
    ) -> TupleView<(C0, C1, C2, C3, C4, C5, C6, C7, C8)> {
        TupleView((c0, c1, c2, c3, c4, c5, c6, c7, c8))
    }
    public static func buildBlock<C0: View, C1: View, C2: View, C3: View, C4: View, C5: View, C6: View, C7: View, C8: View, C9: View>(
        _ c0: C0, _ c1: C1, _ c2: C2, _ c3: C3, _ c4: C4, _ c5: C5, _ c6: C6, _ c7: C7, _ c8: C8, _ c9: C9
    ) -> TupleView<(C0, C1, C2, C3, C4, C5, C6, C7, C8, C9)> {
        TupleView((c0, c1, c2, c3, c4, c5, c6, c7, c8, c9))
    }
    public static func buildOptional<C: View>(_ content: C?) -> C? { content }
    public static func buildEither<TrueContent: View, FalseContent: View>(first: TrueContent) -> _ConditionalContent<TrueContent, FalseContent> {
        _ConditionalContent()
    }
    public static func buildEither<TrueContent: View, FalseContent: View>(second: FalseContent) -> _ConditionalContent<TrueContent, FalseContent> {
        _ConditionalContent()
    }
    public static func buildLimitedAvailability<C: View>(_ content: C) -> AnyView { AnyView() }
}

// ── Scene helper types ─────────────────────────────────────────────────

public struct TupleScene<T0: Scene, T1: Scene>: Scene {
    public init(_ t0: T0, _ t1: T1) {}
}

// ── View types ─────────────────────────────────────────────────────────

public struct EmptyView: View {
    public init() {}
}

public struct AnyView: View {
    public init() {}
    public init<V: View>(_ view: V) {}
    public init<V: View>(erasing view: V) {}
}

public struct TupleView<T>: View {
    public init(_ value: T) {}
}

public struct _ConditionalContent<TrueContent, FalseContent>: View {
    public init() {}
}

public struct Text: View {
    public var key: String
    public init(_ key: String) {
        self.key = key
        _logView("Text", key)
    }
    public init(verbatim: String) {
        self.key = verbatim
        _logView("Text", verbatim)
    }

    // DARWIN ABI OVERRIDES
    @_silgen_name("_$s7SwiftUI4TextVyACxcSyRzlufC")
    public static func _init_text_from_string_protocol(_ s1: UInt64, _ s2: UInt64) -> Text {
        let str = _grafted_translate_darwin_string(s1, s2)
        return Text(str)
    }

    @_silgen_name("_$s7SwiftUI4TextV_9tableName6bundle7commentAcA18LocalizedStringKeyV_SSSgSo8NSBundleCSgs06StaticI0VSgtcfC")
    public static func _init_text_full(_ k1: UInt64, _ k2: UInt64, _ t1: UInt64, _ t2: UInt64, _ b: UInt64, _ c1: UInt64, _ c2: UInt64) -> Text {
        let str = _grafted_translate_darwin_string(k1, k2)
        return Text(str)
    }
}

public struct Label<Title, Icon>: View {
    public init(_ titleKey: LocalizedStringKey, systemImage: String) where Title == Text, Icon == Image {
        _logView("Label", titleKey.stringValue)
    }

    @_silgen_name("_$s7SwiftUI5LabelVA2A4TextVRszAA5ImageVRs_rlE_06systemE0ACyAeGGAA18LocalizedStringKeyV_SStcfC")
    public static func _init_label_system(_ k1: UInt64, _ k2: UInt64, _ i1: UInt64, _ i2: UInt64) -> Label<Text, Image> {
        let str = _grafted_translate_darwin_string(k1, k2)
        let img = _grafted_translate_darwin_string(i1, i2)
        _logView("Label", "\(str) [\(img)]")
        return Label<Text, Image>(LocalizedStringKey(str), systemImage: img)
    }
}

public struct Image: View {
    public init(systemName: String) {
        _logView("Image", systemName)
    }
}

public struct Button<Label: View>: View {
    public init(action: @escaping () -> Void, @ViewBuilder label: () -> Label) {
        let l = label()
        _logView("Button", "")
    }
    public init(_ titleKey: LocalizedStringKey, action: @escaping () -> Void) where Label == Text {
        _logView("Button", titleKey.stringValue)
    }
    public init(_ titleKey: String, action: @escaping () -> Void) where Label == Text {
        _logView("Button", titleKey)
    }
    public init(role: ButtonRole?, action: @escaping () -> Void, @ViewBuilder label: () -> Label) {
        let l = label()
        _logView("Button", "")
    }
}

public struct ButtonRole: Equatable, Sendable {
    public static let destructive = ButtonRole()
    public static let cancel = ButtonRole()
}

public struct Link<Label: View>: View {
    public init(_ titleKey: LocalizedStringKey, destination: URL) where Label == Text {
        _logView("Link", titleKey.stringValue)
    }
    public init(destination: URL, @ViewBuilder label: () -> Label) {
        let _ = label()
        _logView("Link", "")
    }
}

// ── Layout containers ──────────────────────────────────────────────────

public struct VStack<Content: View>: View {
    public init(alignment: HorizontalAlignment = .center, spacing: CGFloat? = nil, @ViewBuilder content: () -> Content) {
        _logView("VStack")
        let _ = content()
    }
}

public struct HStack<Content: View>: View {
    public init(alignment: VerticalAlignment = .center, spacing: CGFloat? = nil, @ViewBuilder content: () -> Content) {
        _logView("HStack")
        let _ = content()
    }
}

public struct ZStack<Content: View>: View {
    public init(alignment: Alignment = .center, @ViewBuilder content: () -> Content) {
        _logView("ZStack")
        let _ = content()
    }
}

public struct LazyVStack<Content: View>: View {
    public init(alignment: HorizontalAlignment = .center, spacing: CGFloat? = nil, pinnedViews: PinnedScrollableViews = .init(), @ViewBuilder content: () -> Content) {
        _logView("LazyVStack")
        let _ = content()
    }
}

public struct PinnedScrollableViews: OptionSet {
    public let rawValue: UInt32
    public init(rawValue: UInt32 = 0) { self.rawValue = rawValue }
    public static let sectionHeaders = PinnedScrollableViews(rawValue: 1)
    public static let sectionFooters = PinnedScrollableViews(rawValue: 2)
}

public struct Group<Content>: View {
    public init(@ViewBuilder content: () -> Content) where Content: View {
        let _ = content()
    }
}

public struct ControlGroup<Content: View>: View {
    public init(@ViewBuilder content: () -> Content) {
        let _ = content()
    }
}

public struct Section<Parent: View, Content: View, Footer: View>: View {
    public init(@ViewBuilder content: () -> Content) where Parent == EmptyView, Footer == EmptyView {
        let _ = content()
    }
    public init(header: Parent, @ViewBuilder content: () -> Content) where Footer == EmptyView {
        let _ = content()
    }
    public init(_ titleKey: LocalizedStringKey, @ViewBuilder content: () -> Content) where Parent == Text, Footer == EmptyView {
        _logView("Section", titleKey.stringValue)
        let _ = content()
    }
}

// ── Scroll / List / ForEach ────────────────────────────────────────────

public struct ScrollView<Content: View>: View {
    public init(_ axes: Axis.Set = .vertical, showsIndicators: Bool = true, @ViewBuilder content: () -> Content) {
        _logView("ScrollView")
        let _ = content()
    }
}

public struct List<SelectionValue, Content: View>: View {
    public init(@ViewBuilder content: () -> Content) where SelectionValue == Never {
        _logView("List")
        let _ = content()
    }
    public init(selection: Binding<Set<SelectionValue>>?, @ViewBuilder content: () -> Content) where SelectionValue: Hashable {
        _logView("List")
        let _ = content()
    }
    public init(selection: Binding<SelectionValue?>?, @ViewBuilder content: () -> Content) where SelectionValue: Hashable {
        _logView("List")
        let _ = content()
    }
}

public struct ForEach<Data, ID, Content>: View where Data: RandomAccessCollection, ID: Hashable {
    public init(_ data: Data, id: KeyPath<Data.Element, ID>, @ViewBuilder content: @escaping (Data.Element) -> Content) {
        _logView("ForEach", "(\(data.count) items)")
    }
}

extension ForEach where Data.Element: Identifiable, ID == Data.Element.ID {
    public init(_ data: Data, @ViewBuilder content: @escaping (Data.Element) -> Content) {
        _logView("ForEach", "(\(data.count) items)")
    }
}

// ── Simple views ───────────────────────────────────────────────────────

public struct Divider: View {
    public init() { _logView("Divider") }
}

public struct Spacer: View {
    public var minLength: CGFloat?
    public init(minLength: CGFloat? = nil) {
        self.minLength = minLength
        _logView("Spacer")
    }
}

public struct Color: View {
    public static let clear = Color()
    public static let black = Color()
    public static let white = Color()
    public static let gray = Color()
    public static let red = Color()
    public static let green = Color()
    public static let blue = Color()
    public static let orange = Color()
    public static let yellow = Color()
    public static let pink = Color()
    public static let purple = Color()
    public static let primary = Color()
    public static let secondary = Color()
    public static let accentColor = Color()
    public init() {}
    public init(_ name: String, bundle: Bundle? = nil) { _logView("Color", name) }
    public init(red: Double, green: Double, blue: Double, opacity: Double = 1.0) {}
    public init(white: Double, opacity: Double = 1.0) {}
    public init(hue: Double, saturation: Double, brightness: Double, opacity: Double = 1.0) {}
}

extension Color {
    public func opacity(_ opacity: Double) -> Color { self }
}

// ── Input controls ─────────────────────────────────────────────────────

public struct TextField<Label: View>: View {
    public init(_ titleKey: LocalizedStringKey, text: Binding<String>) where Label == Text {
        _logView("TextField", titleKey.stringValue)
    }
    public init(_ titleKey: String, text: Binding<String>) where Label == Text {
        _logView("TextField", titleKey)
    }
}

public struct Toggle<Label: View>: View {
    public init(isOn: Binding<Bool>, @ViewBuilder label: () -> Label) {
        let _ = label()
        _logView("Toggle")
    }
    public init(_ titleKey: LocalizedStringKey, isOn: Binding<Bool>) where Label == Text {
        _logView("Toggle", titleKey.stringValue)
    }
}

public struct Picker<Label: View, SelectionValue: Hashable, Content: View>: View {
    public init(selection: Binding<SelectionValue>, @ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        let _ = content()
        let _ = label()
        _logView("Picker")
    }
    public init(_ titleKey: LocalizedStringKey, selection: Binding<SelectionValue>, @ViewBuilder content: () -> Content) where Label == Text {
        _logView("Picker", titleKey.stringValue)
        let _ = content()
    }
}

public struct Stepper<Label: View>: View {
    public init(value: Binding<Int>, in bounds: ClosedRange<Int>, step: Int = 1, @ViewBuilder label: () -> Label) {
        let _ = label()
        _logView("Stepper")
    }
    public init(_ titleKey: LocalizedStringKey, value: Binding<Int>, in bounds: ClosedRange<Int>, step: Int = 1) where Label == Text {
        _logView("Stepper", titleKey.stringValue)
    }
}

public struct LabeledContent<Label: View, Content: View>: View {
    public init(@ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
        let _ = label()
        let _ = content()
        _logView("LabeledContent")
    }
    public init(_ titleKey: LocalizedStringKey, @ViewBuilder content: () -> Content) where Label == Text {
        _logView("LabeledContent", titleKey.stringValue)
        let _ = content()
    }
}

// ── Geometry / Navigation ──────────────────────────────────────────────

public struct GeometryReader<Content: View>: View {
    public init(@ViewBuilder content: @escaping (GeometryProxy) -> Content) {
        _logView("GeometryReader")
    }
}

public struct GeometryProxy {
    public var size: CGSize { CGSize(width: 400, height: 500) }
    public var safeAreaInsets: EdgeInsets { EdgeInsets() }
    public subscript<T>(anchor: Anchor<T>) -> T { fatalError() }
    public func frame(in coordinateSpace: some CoordinateSpaceProtocol) -> CGRect {
        CGRect(x: 0, y: 0, width: 400, height: 500)
    }
}

public struct Anchor<Value> {}

// ── Alignment / Edge / Axis types ──────────────────────────────────────

public struct HorizontalAlignment: Equatable {
    public static let leading = HorizontalAlignment()
    public static let center = HorizontalAlignment()
    public static let trailing = HorizontalAlignment()
}

public struct VerticalAlignment: Equatable {
    public static let top = VerticalAlignment()
    public static let center = VerticalAlignment()
    public static let bottom = VerticalAlignment()
    public static let firstTextBaseline = VerticalAlignment()
    public static let lastTextBaseline = VerticalAlignment()
}

public struct Alignment: Equatable {
    public static let center = Alignment()
    public static let leading = Alignment()
    public static let trailing = Alignment()
    public static let top = Alignment()
    public static let bottom = Alignment()
    public static let topLeading = Alignment()
    public static let topTrailing = Alignment()
    public static let bottomLeading = Alignment()
    public static let bottomTrailing = Alignment()
}

public struct EdgeInsets: Equatable {
    public var top: CGFloat = 0
    public var leading: CGFloat = 0
    public var bottom: CGFloat = 0
    public var trailing: CGFloat = 0
    public init() {}
    public init(top: CGFloat, leading: CGFloat, bottom: CGFloat, trailing: CGFloat) {
        self.top = top; self.leading = leading; self.bottom = bottom; self.trailing = trailing
    }
}

public enum Edge: Int8, CaseIterable {
    case top, leading, bottom, trailing
    public struct Set: OptionSet {
        public let rawValue: Int8
        public init(rawValue: Int8) { self.rawValue = rawValue }
        public static let top = Set(rawValue: 1)
        public static let leading = Set(rawValue: 2)
        public static let bottom = Set(rawValue: 4)
        public static let trailing = Set(rawValue: 8)
        public static let all: Set = [.top, .leading, .bottom, .trailing]
        public static let horizontal: Set = [.leading, .trailing]
        public static let vertical: Set = [.top, .bottom]
    }
}

public enum Axis: Int8, CaseIterable {
    case horizontal, vertical
    public struct Set: OptionSet {
        public let rawValue: Int8
        public init(rawValue: Int8) { self.rawValue = rawValue }
        public static let horizontal = Set(rawValue: 1)
        public static let vertical = Set(rawValue: 2)
    }
}

public struct UnitPoint: Equatable {
    public var x: CGFloat
    public var y: CGFloat
    public init(x: CGFloat = 0, y: CGFloat = 0) { self.x = x; self.y = y }
    public static let zero = UnitPoint()
    public static let center = UnitPoint(x: 0.5, y: 0.5)
    public static let top = UnitPoint(x: 0.5, y: 0)
    public static let bottom = UnitPoint(x: 0.5, y: 1)
    public static let leading = UnitPoint(x: 0, y: 0.5)
    public static let trailing = UnitPoint(x: 1, y: 0.5)
}

// ── Property wrappers ──────────────────────────────────────────────────

@propertyWrapper
public struct State<Value> {
    public var wrappedValue: Value
    public var projectedValue: Binding<Value> {
        Binding(get: { self.wrappedValue }, set: { _ in })
    }
    public init(wrappedValue: Value) { self.wrappedValue = wrappedValue }
    public init(initialValue: Value) { self.wrappedValue = initialValue }
}

@propertyWrapper
public struct Binding<Value> {
    public var wrappedValue: Value
    public var projectedValue: Binding<Value> { self }
    public init(get: @escaping () -> Value, set: @escaping (Value) -> Void) {
        self.wrappedValue = get()
    }
    public static func constant(_ value: Value) -> Binding<Value> {
        Binding(get: { value }, set: { _ in })
    }
}

@propertyWrapper
public struct Environment<Value> {
    public var wrappedValue: Value { fatalError() }
    public init(_ keyPath: KeyPath<EnvironmentValues, Value>) {}
}

@propertyWrapper
public struct AppStorage<Value> {
    public var wrappedValue: Value
    public var projectedValue: Binding<Value> {
        Binding(get: { self.wrappedValue }, set: { _ in })
    }
    public init(wrappedValue: Value, _ key: String) { self.wrappedValue = wrappedValue }
}

@propertyWrapper
public struct ObservedObject<ObjectType: ObservableObject> {
    public var wrappedValue: ObjectType
    public var projectedValue: Wrapper {
        Wrapper(value: wrappedValue)
    }
    public struct Wrapper {
        var value: ObjectType
    }
    public init(wrappedValue: ObjectType) { self.wrappedValue = wrappedValue }
}

@propertyWrapper
public struct StateObject<ObjectType: ObservableObject> {
    public var wrappedValue: ObjectType { fatalError() }
    public init(wrappedValue: @autoclosure @escaping () -> ObjectType) {}
}

@propertyWrapper
public struct FocusState<Value: Hashable> {
    public var wrappedValue: Value
    public var projectedValue: FocusStateBinding<Value> {
        FocusStateBinding(wrappedValue: wrappedValue)
    }
    public init() where Value == Bool { self.wrappedValue = false as! Value }
    public init<T>() where Value == T?, T: Hashable { self.wrappedValue = Optional<T>.none as! Value }
}

public struct FocusStateBinding<Value: Hashable> {
    public var wrappedValue: Value
}

public struct EnvironmentValues {
    public var scenePhase: ScenePhase { .active }
    public var openURL: OpenURLAction { OpenURLAction() }
}

public enum ScenePhase: Equatable {
    case active, inactive, background
}

public struct OpenURLAction {
    public func callAsFunction(_ url: URL) {}
}

// ── Protocols ──────────────────────────────────────────────────────────

public protocol ObservableObject: AnyObject {
    associatedtype ObjectWillChangePublisher
    var objectWillChange: ObjectWillChangePublisher { get }
}

public protocol CoordinateSpaceProtocol {}
public struct LocalCoordinateSpace: CoordinateSpaceProtocol {
    public init() {}
    public static var local: LocalCoordinateSpace { LocalCoordinateSpace() }
}
public struct GlobalCoordinateSpace: CoordinateSpaceProtocol {
    public init() {}
    public static var global: GlobalCoordinateSpace { GlobalCoordinateSpace() }
}

// ── App.main() ─────────────────────────────────────────────────────────

extension App {
    public static func main() {
        let metadata = unsafeBitCast(Self.self, to: UInt.self)
        _grafted_save_conformance(metadata)
        let _ = _NSApplicationMain(metadata, 0)
    }
}

// ── NSApplicationDelegateAdaptor ────────────────────────────────────────

@propertyWrapper
public struct NSApplicationDelegateAdaptor<DelegateType> {
    public var wrappedValue: DelegateType { fatalError() }
    public init(_ delegateType: DelegateType.Type) {}
}

// ── Modifier protocols ─────────────────────────────────────────────────

public protocol ViewModifier {
    associatedtype Body: View
    func body(content: Content) -> Self.Body
    typealias Content = _ViewModifier_Content<Self>
}

public struct _ViewModifier_Content<Modifier: ViewModifier>: View {}

public struct ModifiedContent<Content, Modifier>: View {}

// ── Style types ────────────────────────────────────────────────────────

public struct Font: Equatable {
    public static let largeTitle = Font()
    public static let title = Font()
    public static let title2 = Font()
    public static let title3 = Font()
    public static let headline = Font()
    public static let subheadline = Font()
    public static let body = Font()
    public static let callout = Font()
    public static let footnote = Font()
    public static let caption = Font()
    public static let caption2 = Font()
    public static func system(size: CGFloat) -> Font { Font() }
    public static func system(size: CGFloat, weight: Font.Weight) -> Font { Font() }
    public static func system(size: CGFloat, weight: Font.Weight, design: Font.Design) -> Font { Font() }
    public func bold() -> Font { self }
    public func italic() -> Font { self }
    public func monospaced() -> Font { self }
    public func monospacedDigit() -> Font { self }
    public enum Weight { case ultraLight, thin, light, regular, medium, semibold, bold, heavy, black }
    public enum Design { case `default`, serif, rounded, monospaced }
}

public struct FillStyle {
    public init(eoFill: Bool = false, antialiased: Bool = true) {}
}

public struct RoundedRectangle: Shape {
    public var cornerRadius: CGFloat
    public init(cornerRadius: CGFloat, style: RoundedCornerStyle = .circular) { self.cornerRadius = cornerRadius }
    public func path(in rect: CGRect) -> Path { Path() }
}

public enum RoundedCornerStyle { case circular, continuous }

public protocol Shape: View {
    func path(in rect: CGRect) -> Path
}

public struct Path {
    public init() {}
}

public struct HierarchicalShapeStyle {
    public static let secondary = HierarchicalShapeStyle()
    public static let tertiary = HierarchicalShapeStyle()
    public static let quaternary = HierarchicalShapeStyle()
}

// ── Content margins / Safe area ────────────────────────────────────────

public struct ContentMarginPlacement {
    public static let scrollContent = ContentMarginPlacement()
    public static let scrollIndicators = ContentMarginPlacement()
    public static let automatic = ContentMarginPlacement()
}

public struct SafeAreaRegions: OptionSet {
    public let rawValue: UInt
    public init(rawValue: UInt) { self.rawValue = rawValue }
    public static let container = SafeAreaRegions(rawValue: 1)
    public static let keyboard = SafeAreaRegions(rawValue: 2)
    public static let all: SafeAreaRegions = [.container, .keyboard]
}

// ── Submission ─────────────────────────────────────────────────────────

public struct SubmitTriggers: OptionSet {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let text = SubmitTriggers(rawValue: 1)
    public static let search = SubmitTriggers(rawValue: 2)
}

// ── Key press ──────────────────────────────────────────────────────────

public struct KeyPress {
    public struct Phases: OptionSet {
        public let rawValue: Int
        public init(rawValue: Int) { self.rawValue = rawValue }
        public static let down = Phases(rawValue: 1)
        public static let up = Phases(rawValue: 2)
        public static let all: Phases = [.down, .up]
    }
    public enum Result { case handled, ignored }
}

// ── Gestures ───────────────────────────────────────────────────────────

public struct DragGesture {
    public struct Value {
        public var location: CGPoint { .zero }
        public var startLocation: CGPoint { .zero }
        public var translation: CGSize { .zero }
    }
    public init(minimumDistance: CGFloat = 10, coordinateSpace: some CoordinateSpaceProtocol = LocalCoordinateSpace.local) {}
}

public struct _EndedGesture<T>: View {}
public struct GestureMask: OptionSet {
    public let rawValue: UInt32
    public init(rawValue: UInt32) { self.rawValue = rawValue }
    public static let all = GestureMask(rawValue: 0xFFFF)
    public static let gesture = GestureMask(rawValue: 1)
    public static let subviews = GestureMask(rawValue: 2)
    public static let none = GestureMask(rawValue: 0)
}

// ── Popover / Dialog ───────────────────────────────────────────────────

public enum PopoverAttachmentAnchor {
    case rect(Anchor<CGRect>)
    case point(UnitPoint)
}

// ── Table types ────────────────────────────────────────────────────────

public struct Table<Value, Rows, Columns>: View where Value: Identifiable {
    public init(@ViewBuilder columns: () -> Columns, @ViewBuilder rows: () -> Rows) {
        _logView("Table")
    }
}

public struct TableColumn<RowValue, Sort, Content, Label>: View where RowValue: Identifiable {
    public init(_ titleKey: LocalizedStringKey, @ViewBuilder content: @escaping (RowValue) -> Content) where Sort == Never, Label == Text {
        _logView("TableColumn", titleKey.stringValue)
    }
}

public struct TableForEachContent<Value>: View {}
public struct TupleTableColumnContent<T, Value>: View {}

// ── Tab ────────────────────────────────────────────────────────────────

public struct Tab<Title, Content, SelectionValue>: View {
    public init(_ titleKey: LocalizedStringKey, systemImage: String, @ViewBuilder content: () -> Content) where Title == Label<Text, Image>, SelectionValue == Never {
        _logView("Tab", titleKey.stringValue)
        let _ = content()
    }
}

// ── Subscription / Transaction ─────────────────────────────────────────

public struct Subscription {}

public struct Transaction {
    public var disablesAnimations: Bool = false
    public init() {}
}

// ── Animation ──────────────────────────────────────────────────────────

public struct Animation: Equatable {
    public static let `default` = Animation()
    public static func easeInOut(duration: Double) -> Animation { Animation() }
    public static func easeIn(duration: Double) -> Animation { Animation() }
    public static let spring = Animation()
}

// ── Style protocols ────────────────────────────────────────────────────

public struct PlainButtonStyle {
    public init() {}
}

public struct BorderlessButtonStyle {
    public init() {}
}

public struct PlainTextFieldStyle {
    public init() {}
}

// ── View modifier extensions ───────────────────────────────────────────

extension View {
    public func font(_ font: Font?) -> some View { self }
    public func foregroundColor(_ color: Color?) -> some View { self }
    public func foregroundStyle<S>(_ style: S) -> some View { self }
    public func background<V: View>(_ background: V, alignment: Alignment = .center) -> some View { self }
    public func background<S>(_ style: S, ignoresSafeAreaEdges: Edge.Set = .all) -> some View { self }
    public func padding(_ edges: Edge.Set = .all, _ length: CGFloat? = nil) -> some View { self }
    public func padding(_ length: CGFloat) -> some View { self }
    public func frame(width: CGFloat? = nil, height: CGFloat? = nil, alignment: Alignment = .center) -> some View { self }
    public func frame(minWidth: CGFloat? = nil, idealWidth: CGFloat? = nil, maxWidth: CGFloat? = nil,
                      minHeight: CGFloat? = nil, idealHeight: CGFloat? = nil, maxHeight: CGFloat? = nil,
                      alignment: Alignment = .center) -> some View { self }
    public func opacity(_ opacity: Double) -> some View { self }
    public func clipShape<S: Shape>(_ shape: S, style: FillStyle = FillStyle()) -> some View { self }
    public func overlay<V: View>(_ overlay: V, alignment: Alignment = .center) -> some View { self }
    public func offset(x: CGFloat = 0, y: CGFloat = 0) -> some View { self }
    public func fixedSize(horizontal: Bool = true, vertical: Bool = true) -> some View { self }
    public func onAppear(perform action: (() -> Void)? = nil) -> some View { self }
    public func onDisappear(perform action: (() -> Void)? = nil) -> some View { self }
    public func onChange<V: Equatable>(of value: V, perform action: @escaping (V) -> Void) -> some View { self }
    public func onChange<V: Equatable>(of value: V, initial: Bool = false, _ action: @escaping () -> Void) -> some View { self }
    public func disabled(_ disabled: Bool) -> some View { self }
    public func hidden() -> some View { self }
    public func id<ID: Hashable>(_ id: ID) -> some View { self }
    public func tag<V: Hashable>(_ tag: V) -> some View { self }
    public func help(_ text: Text) -> some View { self }
    public func help(_ textKey: LocalizedStringKey) -> some View { self }
    public func accessibilityIdentifier(_ identifier: String) -> some View { self }
    public func accessibilityLabel(_ label: Text) -> some View { self }
    public func accessibilityLabel(_ textKey: LocalizedStringKey) -> some View { self }
    public func onSubmit(of triggers: SubmitTriggers = .text, _ action: @escaping () -> Void) -> some View { self }
    public func contentMargins(_ edges: Edge.Set = .all, _ length: CGFloat? = nil, for placement: ContentMarginPlacement = .automatic) -> some View { self }
    public func ignoresSafeArea(_ regions: SafeAreaRegions = .all, edges: Edge.Set = .all) -> some View { self }
    public func buttonStyle<S>(_ style: S) -> some View { self }
    public func textFieldStyle<S>(_ style: S) -> some View { self }
    public func focused<Value: Hashable>(_ binding: FocusStateBinding<Value>, equals value: Value) -> some View { self }
    public func focused(_ condition: FocusStateBinding<Bool>) -> some View { self }
    public func onKeyPress(phases: KeyPress.Phases = .down, action: @escaping () -> KeyPress.Result) -> some View { self }
    public func gesture<T>(_ gesture: T, including mask: GestureMask = .all) -> some View { self }
    public func highPriorityGesture<T>(_ gesture: T, including mask: GestureMask = .all) -> some View { self }
    public func popover<Content: View>(isPresented: Binding<Bool>, attachmentAnchor: PopoverAttachmentAnchor = .point(.bottom), arrowEdge: Edge = .top, @ViewBuilder content: @escaping () -> Content) -> some View { self }
    public func confirmationDialog<A: View>(_ titleKey: LocalizedStringKey, isPresented: Binding<Bool>, titleVisibility: Visibility = .automatic, @ViewBuilder actions: () -> A) -> some View { self }
    public func fileImporter(isPresented: Binding<Bool>, allowedContentTypes: [Any], onCompletion: @escaping (Result<URL, Error>) -> Void) -> some View { self }
    public func fileDialogDefaultDirectory(_ defaultDirectory: URL?) -> some View { self }
    public func dialogSuppressionToggle(isSuppressed: Binding<Bool>) -> some View { self }
    public func truncationMode(_ mode: Text.TruncationMode) -> some View { self }
    public func alignmentGuide(_ g: HorizontalAlignment, computeValue: @escaping (ViewDimensions) -> CGFloat) -> some View { self }
    public func alignmentGuide(_ g: VerticalAlignment, computeValue: @escaping (ViewDimensions) -> CGFloat) -> some View { self }
    public func onHover(perform action: @escaping (Bool) -> Void) -> some View { self }
    public func task(priority: TaskPriority = .userInitiated, _ action: @escaping () async -> Void) -> some View { self }
    public func task<T: Equatable>(id value: T, priority: TaskPriority = .userInitiated, _ action: @escaping () async -> Void) -> some View { self }
    public func animation<V: Equatable>(_ animation: Animation?, value: V) -> some View { self }
    public func transition(_ t: AnyTransition) -> some View { self }
    public func toolbar<Content: View>(@ViewBuilder content: () -> Content) -> some View { self }
    public func navigationTitle(_ title: Text) -> some View { self }
    public func navigationTitle(_ titleKey: LocalizedStringKey) -> some View { self }
    public func environment<V>(_ keyPath: WritableKeyPath<EnvironmentValues, V>, _ value: V) -> some View { self }
    public func environmentObject<T: ObservableObject>(_ object: T) -> some View { self }
    public func preference<K>(key: K.Type, value: K.Value) -> some View where K: PreferenceKey { self }
    public func onPreferenceChange<K: PreferenceKey>(_ key: K.Type, perform action: @escaping (K.Value) -> Void) -> some View { self }
    public func transformPreference<K: PreferenceKey>(_ key: K.Type, _ callback: @escaping (inout K.Value) -> Void) -> some View { self }
}

// More Text extensions
extension Text {
    public enum TruncationMode { case head, tail, middle }
    public func font(_ font: Font?) -> Text { self }
    public func bold() -> Text { self }
    public func italic() -> Text { self }
    public func foregroundColor(_ color: Color?) -> Text { self }
    public func foregroundStyle<S>(_ style: S) -> Text { self }
    public func strikethrough(_ active: Bool = true, color: Color? = nil) -> Text { self }
    public func underline(_ active: Bool = true, color: Color? = nil) -> Text { self }
    public func lineLimit(_ number: Int?) -> some View { self }
    public static func + (lhs: Text, rhs: Text) -> Text { Text(lhs.key + rhs.key) }
}

// DragGesture extensions
extension DragGesture {
    public func onEnded(_ action: @escaping (DragGesture.Value) -> Void) -> _EndedGesture<DragGesture> {
        _EndedGesture<DragGesture>()
    }
}

public struct ViewDimensions {
    public var width: CGFloat { 0 }
    public var height: CGFloat { 0 }
    public subscript(guide: HorizontalAlignment) -> CGFloat { 0 }
    public subscript(guide: VerticalAlignment) -> CGFloat { 0 }
}

public enum Visibility { case automatic, visible, hidden }

public struct AnyTransition {
    public static let opacity = AnyTransition()
    public static let slide = AnyTransition()
    public static let identity = AnyTransition()
    public static func move(edge: Edge) -> AnyTransition { AnyTransition() }
    public func combined(with other: AnyTransition) -> AnyTransition { AnyTransition() }
}

// ── PreferenceKey ──────────────────────────────────────────────────────

public protocol PreferenceKey {
    associatedtype Value
    static var defaultValue: Value { get }
    static func reduce(value: inout Value, nextValue: () -> Value)
}

// ── CGRect convenience ─────────────────────────────────────────────────

extension CGRect {
    public init(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
        self.init(origin: CGPoint(x: x, y: y), size: CGSize(width: width, height: height))
    }
}

// ── Bindable (SwiftData interop) ───────────────────────────────────────

@propertyWrapper
public struct Bindable<Value> {
    public var wrappedValue: Value
    public init(wrappedValue: Value) { self.wrappedValue = wrappedValue }
}

// ── NSHostingController / NSHostingView ─────────────────────────────────

public class NSHostingController<Content> {
    public init(rootView: Content) {}
    public init?(coder: NSCoder, rootView: Content) { return nil }
}

public class NSHostingView<Content> {
    public init(rootView: Content) {}
}
