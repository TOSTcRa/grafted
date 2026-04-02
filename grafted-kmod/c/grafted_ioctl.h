/* SPDX-License-Identifier: GPL-2.0 */
/*
 * grafted_ioctl.h — ioctl definitions for /dev/grafted
 *
 * These must match the definitions in crates/grafted-kernel/src/ioctl.rs
 */

#ifndef _GRAFTED_IOCTL_H
#define _GRAFTED_IOCTL_H

#include <linux/ioctl.h>
#include <linux/types.h>

/* ioctl magic number — 'G' for Grafted */
#define GRAFTED_IOC_MAGIC 'G'

/* Ping the module, returns version in arg */
#define GRAFTED_IOC_PING        _IOR(GRAFTED_IOC_MAGIC, 0, __u64)

/* Register current process as a Darwin emulation target */
#define GRAFTED_IOC_REGISTER    _IOW(GRAFTED_IOC_MAGIC, 1, __u32)

/* Unregister current process */
#define GRAFTED_IOC_UNREGISTER  _IO(GRAFTED_IOC_MAGIC, 2)

/* Translate a Mach trap — pass trap number and args, get result */
struct grafted_mach_trap {
    __u32 trap_number;
    __u64 args[6];
    __s64 result;
};
#define GRAFTED_IOC_MACH_TRAP   _IOWR(GRAFTED_IOC_MAGIC, 3, struct grafted_mach_trap)

/* Module version */
#define GRAFTED_VERSION 0x00010000ULL  /* 0.1.0 */

#define GRAFTED_MAX_NR 3

#endif /* _GRAFTED_IOCTL_H */
