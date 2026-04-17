//! CG geometry types - CGPoint, CGSize, CGRect, CGAffineTransform.

pub type CGFloat = f64;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CGAffineTransform {
    pub a: CGFloat, pub b: CGFloat,
    pub c: CGFloat, pub d: CGFloat,
    pub tx: CGFloat, pub ty: CGFloat,
}

impl Default for CGAffineTransform {
    fn default() -> Self { Self::identity() }
}

impl CGAffineTransform {
    pub const fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 }
    }
}

// ---- C API ----

#[unsafe(no_mangle)] pub extern "C" fn CGPointMake(x: CGFloat, y: CGFloat) -> CGPoint { CGPoint { x, y } }
#[unsafe(no_mangle)] pub extern "C" fn CGSizeMake(w: CGFloat, h: CGFloat) -> CGSize { CGSize { width: w, height: h } }
#[unsafe(no_mangle)] pub extern "C" fn CGRectMake(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat) -> CGRect {
    CGRect { origin: CGPoint { x, y }, size: CGSize { width: w, height: h } }
}

#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMinX(r: CGRect) -> CGFloat { r.origin.x }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMinY(r: CGRect) -> CGFloat { r.origin.y }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMaxX(r: CGRect) -> CGFloat { r.origin.x + r.size.width }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMaxY(r: CGRect) -> CGFloat { r.origin.y + r.size.height }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMidX(r: CGRect) -> CGFloat { r.origin.x + r.size.width / 2.0 }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetMidY(r: CGRect) -> CGFloat { r.origin.y + r.size.height / 2.0 }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetWidth(r: CGRect) -> CGFloat { r.size.width }
#[unsafe(no_mangle)] pub extern "C" fn CGRectGetHeight(r: CGRect) -> CGFloat { r.size.height }
#[unsafe(no_mangle)] pub extern "C" fn CGRectIsEmpty(r: CGRect) -> bool { r.size.width <= 0.0 || r.size.height <= 0.0 }

#[unsafe(no_mangle)]
pub extern "C" fn CGRectIntersection(r1: CGRect, r2: CGRect) -> CGRect {
    let x = r1.origin.x.max(r2.origin.x);
    let y = r1.origin.y.max(r2.origin.y);
    let x2 = (r1.origin.x + r1.size.width).min(r2.origin.x + r2.size.width);
    let y2 = (r1.origin.y + r1.size.height).min(r2.origin.y + r2.size.height);
    if x2 > x && y2 > y {
        CGRect { origin: CGPoint { x, y }, size: CGSize { width: x2 - x, height: y2 - y } }
    } else {
        CGRect::default()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn CGRectUnion(r1: CGRect, r2: CGRect) -> CGRect {
    let x = r1.origin.x.min(r2.origin.x);
    let y = r1.origin.y.min(r2.origin.y);
    let x2 = (r1.origin.x + r1.size.width).max(r2.origin.x + r2.size.width);
    let y2 = (r1.origin.y + r1.size.height).max(r2.origin.y + r2.size.height);
    CGRect { origin: CGPoint { x, y }, size: CGSize { width: x2 - x, height: y2 - y } }
}

#[unsafe(no_mangle)]
pub extern "C" fn CGRectContainsPoint(r: CGRect, p: CGPoint) -> bool {
    p.x >= r.origin.x && p.x < r.origin.x + r.size.width &&
    p.y >= r.origin.y && p.y < r.origin.y + r.size.height
}

#[unsafe(no_mangle)]
pub extern "C" fn CGRectEqualToRect(r1: CGRect, r2: CGRect) -> bool { r1 == r2 }
#[unsafe(no_mangle)]
pub extern "C" fn CGPointEqualToPoint(p1: CGPoint, p2: CGPoint) -> bool { p1 == p2 }
#[unsafe(no_mangle)]
pub extern "C" fn CGSizeEqualToSize(s1: CGSize, s2: CGSize) -> bool { s1 == s2 }

#[unsafe(no_mangle)] pub static CGPointZero: CGPoint = CGPoint { x: 0.0, y: 0.0 };
#[unsafe(no_mangle)] pub static CGSizeZero: CGSize = CGSize { width: 0.0, height: 0.0 };
#[unsafe(no_mangle)] pub static CGRectZero: CGRect = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 0.0, height: 0.0 } };
#[unsafe(no_mangle)] pub static CGRectNull: CGRect = CGRect { origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY }, size: CGSize { width: 0.0, height: 0.0 } };
#[unsafe(no_mangle)] pub static CGAffineTransformIdentity: CGAffineTransform = CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };
