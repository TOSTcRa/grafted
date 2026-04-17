//! Foundation framework - NSString, NSArray, NSDictionary, NSBundle, NSNotificationCenter.

pub mod string;
pub mod bundle;
pub mod notification;
pub mod register;

pub fn register_classes() {
    register::register_all();
}
