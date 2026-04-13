//! Mach IPC emulation — in-process port namespace, message queues, and port management.
//!
//! Darwin's Mach IPC is the foundation for nearly all system services: launchd,
//! WindowServer, IOKit, XPC, and Core Foundation's CFRunLoop. This module provides
//! a userspace emulation of the Mach port namespace and messaging primitives.
//!
//! Architecture:
//! - One global `PortSpace` per process (Darwin tasks have one IPC space each)
//! - Ports have receive rights (one holder) and send rights (ref-counted)
//! - Special ports are pre-allocated: task_self, thread_self, host, bootstrap
//! - `mach_msg` delivers messages through in-process queues (no kernel needed)

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

// ---- Mach constants ----

pub const MACH_PORT_NULL: u32 = 0;
pub const MACH_PORT_DEAD: u32 = 0xFFFF_FFFF;

// Port right types
pub const MACH_PORT_RIGHT_SEND: u32 = 0;
pub const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
pub const MACH_PORT_RIGHT_SEND_ONCE: u32 = 2;
pub const MACH_PORT_RIGHT_PORT_SET: u32 = 3;
pub const MACH_PORT_RIGHT_DEAD_NAME: u32 = 4;

// Kern return codes
pub const KERN_SUCCESS: i32 = 0;
pub const KERN_INVALID_ARGUMENT: i32 = 4;
pub const KERN_NO_SPACE: i32 = 3;
pub const KERN_INVALID_NAME: i32 = 15;
pub const KERN_INVALID_RIGHT: i32 = 16;
pub const KERN_FAILURE: i32 = 5;

// mach_msg options
pub const MACH_SEND_MSG: i32 = 0x0000_0001;
pub const MACH_RCV_MSG: i32 = 0x0000_0002;
pub const MACH_SEND_TIMEOUT: i32 = 0x0000_0010;
pub const MACH_RCV_TIMEOUT: i32 = 0x0000_0100;

// mach_msg_type_name_t for port right transfer
pub const MACH_MSG_TYPE_MOVE_RECEIVE: u32 = 16;
pub const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
pub const MACH_MSG_TYPE_MOVE_SEND_ONCE: u32 = 18;
pub const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
pub const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
pub const MACH_MSG_TYPE_MAKE_SEND_ONCE: u32 = 21;

// mach_msg return codes
pub const MACH_MSG_SUCCESS: i32 = 0;
pub const MACH_SEND_INVALID_DEST: i32 = 0x1000_0002;
pub const MACH_SEND_TIMED_OUT: i32 = 0x1000_0003;
pub const MACH_RCV_INVALID_NAME: i32 = 0x1000_0002;
pub const MACH_RCV_TIMED_OUT: i32 = 0x1000_0003;
pub const MACH_RCV_TOO_LARGE: i32 = 0x1000_0004;

// Well-known special port names
pub const SPECIAL_PORT_TASK_SELF: u32 = 0x103;
pub const SPECIAL_PORT_THREAD_SELF: u32 = 0x203;
pub const SPECIAL_PORT_HOST_SELF: u32 = 0x303;
pub const SPECIAL_PORT_REPLY: u32 = 0x307;
pub const SPECIAL_PORT_BOOTSTRAP: u32 = 0x40b;

// ---- Mach message header (matches Darwin's mach_msg_header_t) ----

#[repr(C)]
#[derive(Clone, Debug)]
pub struct MachMsgHeader {
    pub msgh_bits: u32,
    pub msgh_size: u32,
    pub msgh_remote_port: u32,
    pub msgh_local_port: u32,
    pub msgh_voucher_port: u32,
    pub msgh_id: i32,
}

// ---- Port internals ----

#[derive(Debug)]
struct Port {
    send_count: u32,
    has_receive: bool,
    is_dead: bool,
    queue: VecDeque<MachMessage>,
}

#[derive(Clone, Debug)]
struct MachMessage {
    header: MachMsgHeader,
    body: Vec<u8>, // inline body after header
}

impl Port {
    fn new_with_receive() -> Self {
        Self {
            send_count: 0,
            has_receive: true,
            is_dead: false,
            queue: VecDeque::new(),
        }
    }

    fn new_send_only() -> Self {
        Self {
            send_count: 1,
            has_receive: false,
            is_dead: false,
            queue: VecDeque::new(),
        }
    }

    fn new_with_send_receive() -> Self {
        Self {
            send_count: 1,
            has_receive: true,
            is_dead: false,
            queue: VecDeque::new(),
        }
    }
}

// ---- Port namespace ----

struct PortSpace {
    ports: HashMap<u32, Port>,
    next_name: u32,
}

impl PortSpace {
    fn new() -> Self {
        let mut ps = Self {
            ports: HashMap::new(),
            next_name: 0x1000, // user ports start here
        };

        // Pre-allocate special ports with send+receive rights
        ps.ports.insert(SPECIAL_PORT_TASK_SELF, Port::new_with_send_receive());
        ps.ports.insert(SPECIAL_PORT_THREAD_SELF, Port::new_with_send_receive());
        ps.ports.insert(SPECIAL_PORT_HOST_SELF, Port::new_with_send_receive());
        ps.ports.insert(SPECIAL_PORT_REPLY, Port::new_with_send_receive());
        ps.ports.insert(SPECIAL_PORT_BOOTSTRAP, Port::new_with_send_receive());

        ps
    }

    fn allocate(&mut self, right: u32) -> Result<u32, i32> {
        let name = self.next_name;
        self.next_name += 1;
        if self.next_name > 0xFFFF_FFFE {
            return Err(KERN_NO_SPACE);
        }

        let port = match right {
            MACH_PORT_RIGHT_RECEIVE => Port::new_with_receive(),
            MACH_PORT_RIGHT_SEND => Port::new_send_only(),
            MACH_PORT_RIGHT_PORT_SET => Port::new_with_receive(), // simplified
            MACH_PORT_RIGHT_DEAD_NAME => {
                let mut p = Port::new_send_only();
                p.is_dead = true;
                p
            }
            _ => return Err(KERN_INVALID_ARGUMENT),
        };

        self.ports.insert(name, port);
        Ok(name)
    }

    fn deallocate(&mut self, name: u32) -> i32 {
        match self.ports.get_mut(&name) {
            Some(port) => {
                if port.send_count > 0 {
                    port.send_count -= 1;
                }
                if port.send_count == 0 && !port.has_receive {
                    self.ports.remove(&name);
                }
                KERN_SUCCESS
            }
            None => KERN_INVALID_NAME,
        }
    }

    fn insert_right(&mut self, name: u32, _poly: u32, poly_type: u32) -> i32 {
        // If name doesn't exist and we're inserting a receive right, create it
        if !self.ports.contains_key(&name) {
            match poly_type {
                MACH_MSG_TYPE_MAKE_SEND | MACH_MSG_TYPE_COPY_SEND | MACH_MSG_TYPE_MOVE_SEND => {
                    // Create a send-only entry
                    self.ports.insert(name, Port::new_send_only());
                    return KERN_SUCCESS;
                }
                MACH_MSG_TYPE_MOVE_RECEIVE => {
                    self.ports.insert(name, Port::new_with_receive());
                    return KERN_SUCCESS;
                }
                _ => return KERN_INVALID_ARGUMENT,
            }
        }

        let port = self.ports.get_mut(&name).unwrap();
        match poly_type {
            MACH_MSG_TYPE_MAKE_SEND | MACH_MSG_TYPE_COPY_SEND => {
                port.send_count += 1;
                KERN_SUCCESS
            }
            MACH_MSG_TYPE_MOVE_SEND => {
                port.send_count += 1;
                KERN_SUCCESS
            }
            MACH_MSG_TYPE_MOVE_RECEIVE => {
                port.has_receive = true;
                KERN_SUCCESS
            }
            MACH_MSG_TYPE_MAKE_SEND_ONCE | MACH_MSG_TYPE_MOVE_SEND_ONCE => {
                // send-once rights are consumed on use; just succeed
                KERN_SUCCESS
            }
            _ => KERN_INVALID_ARGUMENT,
        }
    }

    fn mod_refs(&mut self, name: u32, right: u32, delta: i32) -> i32 {
        match self.ports.get_mut(&name) {
            Some(port) => {
                match right {
                    MACH_PORT_RIGHT_SEND => {
                        let new_count = port.send_count as i32 + delta;
                        if new_count < 0 {
                            port.send_count = 0;
                        } else {
                            port.send_count = new_count as u32;
                        }
                    }
                    MACH_PORT_RIGHT_RECEIVE => {
                        if delta < 0 {
                            port.has_receive = false;
                        }
                    }
                    _ => {}
                }
                // Clean up if no rights remain
                if port.send_count == 0 && !port.has_receive {
                    self.ports.remove(&name);
                }
                KERN_SUCCESS
            }
            None => KERN_INVALID_NAME,
        }
    }

    fn send_msg(&mut self, dest: u32, msg: MachMessage) -> i32 {
        match self.ports.get_mut(&dest) {
            Some(port) if !port.is_dead => {
                port.queue.push_back(msg);
                MACH_MSG_SUCCESS
            }
            _ => MACH_SEND_INVALID_DEST,
        }
    }

    fn recv_msg(&mut self, port_name: u32) -> Option<MachMessage> {
        self.ports.get_mut(&port_name)?.queue.pop_front()
    }

    fn port_type(&self, name: u32) -> u32 {
        match self.ports.get(&name) {
            Some(port) => {
                let mut ptype = 0u32;
                if port.has_receive { ptype |= 1 << 17; } // MACH_PORT_TYPE_RECEIVE
                if port.send_count > 0 { ptype |= 1 << 16; } // MACH_PORT_TYPE_SEND
                if port.is_dead { ptype |= 1 << 20; } // MACH_PORT_TYPE_DEAD_NAME
                ptype
            }
            None => 0,
        }
    }
}

// ---- Global port space ----

static PORT_SPACE: OnceLock<Mutex<PortSpace>> = OnceLock::new();

fn port_space() -> &'static Mutex<PortSpace> {
    PORT_SPACE.get_or_init(|| Mutex::new(PortSpace::new()))
}

// ---- Public API (called from shims) ----

/// mach_port_allocate(task, right, &name) → kern_return_t
pub fn port_allocate(right: u32) -> Result<u32, i32> {
    port_space().lock().unwrap().allocate(right)
}

/// mach_port_deallocate(task, name) → kern_return_t
pub fn port_deallocate(name: u32) -> i32 {
    port_space().lock().unwrap().deallocate(name)
}

/// mach_port_insert_right(task, name, poly, polyPoly) → kern_return_t
pub fn port_insert_right(name: u32, poly: u32, poly_type: u32) -> i32 {
    port_space().lock().unwrap().insert_right(name, poly, poly_type)
}

/// mach_port_mod_refs(task, name, right, delta) → kern_return_t
pub fn port_mod_refs(name: u32, right: u32, delta: i32) -> i32 {
    port_space().lock().unwrap().mod_refs(name, right, delta)
}

/// mach_port_type(task, name, &ptype) → kern_return_t
pub fn port_type(name: u32) -> (i32, u32) {
    let ptype = port_space().lock().unwrap().port_type(name);
    if ptype == 0 { (KERN_INVALID_NAME, 0) } else { (KERN_SUCCESS, ptype) }
}

/// mach_msg — send and/or receive a message.
///
/// This is the core Mach IPC primitive. `msg` points to a mach_msg_header_t
/// followed by inline body data.
pub unsafe fn mach_msg(
    msg: *mut MachMsgHeader,
    option: i32,
    send_size: u32,
    rcv_size: u32,
    rcv_name: u32,
    _timeout: u32,
    _notify: u32,
) -> i32 {
    if msg.is_null() {
        return KERN_INVALID_ARGUMENT;
    }

    let mut ps = port_space().lock().unwrap();

    // Send phase
    if option & MACH_SEND_MSG != 0 {
        let hdr = unsafe { &*msg };
        let dest = hdr.msgh_remote_port;

        // Copy full message (header + body)
        let total_size = send_size as usize;
        let body_size = total_size.saturating_sub(std::mem::size_of::<MachMsgHeader>());
        let body = if body_size > 0 {
            let body_ptr = unsafe { (msg as *const u8).add(std::mem::size_of::<MachMsgHeader>()) };
            unsafe { std::slice::from_raw_parts(body_ptr, body_size) }.to_vec()
        } else {
            Vec::new()
        };

        let mach_msg = MachMessage {
            header: hdr.clone(),
            body,
        };

        let ret = ps.send_msg(dest, mach_msg);
        if ret != MACH_MSG_SUCCESS {
            return ret;
        }
    }

    // Receive phase
    if option & MACH_RCV_MSG != 0 {
        let port = if option & MACH_SEND_MSG != 0 {
            // Combined send+receive: receive on the reply port
            let hdr = unsafe { &*msg };
            hdr.msgh_local_port
        } else {
            rcv_name
        };

        match ps.recv_msg(port) {
            Some(mach_msg) => {
                // Copy header back
                unsafe { *msg = mach_msg.header };

                // Copy body
                let max_body = rcv_size as usize - std::mem::size_of::<MachMsgHeader>();
                let copy_len = mach_msg.body.len().min(max_body);
                if copy_len > 0 {
                    let body_ptr = unsafe {
                        (msg as *mut u8).add(std::mem::size_of::<MachMsgHeader>())
                    };
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            mach_msg.body.as_ptr(),
                            body_ptr,
                            copy_len,
                        );
                    }
                }
                unsafe { (*msg).msgh_size = (std::mem::size_of::<MachMsgHeader>() + copy_len) as u32 };
            }
            None => {
                if option & MACH_RCV_TIMEOUT != 0 {
                    return MACH_RCV_TIMED_OUT;
                }
                // No message available — for now, return timed out
                // (real implementation would block)
                return MACH_RCV_TIMED_OUT;
            }
        }
    }

    MACH_MSG_SUCCESS
}

/// Get a special port name.
pub fn task_self_port() -> u32 { SPECIAL_PORT_TASK_SELF }
pub fn thread_self_port() -> u32 { SPECIAL_PORT_THREAD_SELF }
pub fn host_self_port() -> u32 { SPECIAL_PORT_HOST_SELF }
pub fn reply_port() -> u32 { SPECIAL_PORT_REPLY }
pub fn bootstrap_port() -> u32 { SPECIAL_PORT_BOOTSTRAP }
