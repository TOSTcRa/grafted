//! NSMenu + NSMenuItem - application menus.

use crate::cf::string::CFStringInner;

const MENU_DATA_OFFSET: usize = 16;

#[repr(C)]
pub struct NSMenuData {
    pub title: [u8; 256],
    pub title_len: usize,
    pub items: [*mut u8; 64], // NSMenuItem pointers
    pub item_count: u32,
}

#[repr(C)]
pub struct NSMenuItemData {
    pub title: [u8; 256],
    pub title_len: usize,
    pub key_equiv: [u8; 16],
    pub key_equiv_len: usize,
    pub action: *mut u8,  // SEL
    pub target: *mut u8,  // id
    pub tag: i64,
    pub enabled: bool,
    pub submenu: *mut u8, // NSMenu*
    pub separator: bool,
}

unsafe fn menu_data(obj: *mut u8) -> *mut NSMenuData { unsafe {
    obj.add(MENU_DATA_OFFSET) as *mut NSMenuData
}}
unsafe fn item_data(obj: *mut u8) -> *mut NSMenuItemData { unsafe {
    obj.add(MENU_DATA_OFFSET) as *mut NSMenuItemData
}}

// ---- NSMenu methods ----

pub unsafe extern "C" fn ns_menu_init_with_title(
    self_: *mut u8, _sel: *mut u8, title: *mut u8,
) -> *mut u8 {
    let data = unsafe { &mut *menu_data(self_) };
    data.item_count = 0;
    if !title.is_null() {
        let cf = title as *const CFStringInner;
        let bytes = unsafe { &(*cf).bytes };
        let len = bytes.len().min(255);
        data.title[..len].copy_from_slice(&bytes[..len]);
        data.title_len = len;
    }
    self_
}

pub unsafe extern "C" fn ns_menu_add_item(
    self_: *mut u8, _sel: *mut u8, item: *mut u8,
) {
    if self_.is_null() || item.is_null() { return; }
    let data = unsafe { &mut *menu_data(self_) };
    let idx = data.item_count as usize;
    if idx < 64 {
        data.items[idx] = item;
        data.item_count += 1;
    }
}

pub unsafe extern "C" fn ns_menu_item_count(self_: *mut u8, _sel: *mut u8) -> i64 {
    if self_.is_null() { return 0; }
    unsafe { (*menu_data(self_)).item_count as i64 }
}

pub unsafe extern "C" fn ns_menu_set_submenu(
    _self: *mut u8, _sel: *mut u8, submenu: *mut u8, _item: *mut u8,
) {
    // Stub: associate submenu with item
    if !_item.is_null() {
        let idata = unsafe { &mut *item_data(_item) };
        idata.submenu = submenu;
    }
}

// ---- NSMenuItem methods ----

pub unsafe extern "C" fn ns_menu_item_init(
    self_: *mut u8, _sel: *mut u8,
    title: *mut u8, action: *mut u8, key_equiv: *mut u8,
) -> *mut u8 {
    let data = unsafe { &mut *item_data(self_) };
    data.action = action;
    data.enabled = true;
    data.separator = false;
    if !title.is_null() {
        let cf = title as *const CFStringInner;
        let bytes = unsafe { &(*cf).bytes };
        let len = bytes.len().min(255);
        data.title[..len].copy_from_slice(&bytes[..len]);
        data.title_len = len;
    }
    if !key_equiv.is_null() {
        let cf = key_equiv as *const CFStringInner;
        let bytes = unsafe { &(*cf).bytes };
        let len = bytes.len().min(15);
        data.key_equiv[..len].copy_from_slice(&bytes[..len]);
        data.key_equiv_len = len;
    }
    self_
}

pub unsafe extern "C" fn ns_menu_item_separator(
    _cls: *mut u8, _sel: *mut u8,
) -> *mut u8 {
    let obj = unsafe { libc::calloc(1, 512) } as *mut u8;
    let data = unsafe { &mut *item_data(obj) };
    data.separator = true;
    obj
}

pub unsafe extern "C" fn ns_menu_item_set_target(
    self_: *mut u8, _sel: *mut u8, target: *mut u8,
) {
    if !self_.is_null() { unsafe { (*item_data(self_)).target = target }; }
}

pub unsafe extern "C" fn ns_menu_item_set_submenu(
    self_: *mut u8, _sel: *mut u8, submenu: *mut u8,
) {
    if !self_.is_null() { unsafe { (*item_data(self_)).submenu = submenu }; }
}

pub unsafe extern "C" fn ns_menu_item_set_enabled(
    self_: *mut u8, _sel: *mut u8, flag: bool,
) {
    if !self_.is_null() { unsafe { (*item_data(self_)).enabled = flag }; }
}

pub unsafe extern "C" fn ns_menu_item_set_tag(
    self_: *mut u8, _sel: *mut u8, tag: i64,
) {
    if !self_.is_null() { unsafe { (*item_data(self_)).tag = tag }; }
}
