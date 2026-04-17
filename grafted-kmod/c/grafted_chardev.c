// SPDX-License-Identifier: GPL-2.0
/*
 * grafted_chardev.c - /dev/grafted misc character device
 *
 * Provides ioctl interface for userspace to request Mach trap handling
 * and syscall translation from the kernel module.
 */

#include <linux/miscdevice.h>
#include <linux/fs.h>
#include <linux/kernel.h>
#include <linux/uaccess.h>
#include <linux/slab.h>

#include "grafted_chardev.h"
#include "grafted_ioctl.h"

static int grafted_open(struct inode *inode, struct file *file)
{
	pr_debug("grafted: device opened by pid %d\n", current->pid);
	return 0;
}

static int grafted_release(struct inode *inode, struct file *file)
{
	pr_debug("grafted: device closed by pid %d\n", current->pid);
	return 0;
}

static long dispatch_mach_trap(struct grafted_mach_trap *trap)
{
	switch (trap->trap_number) {
	case 26: /* mach_reply_port */
		/* Just return a fake port name based on pid for now */
		trap->result = 0x1000 + current->pid;
		return 0;

	case 27: /* thread_self_trap */
		/* Return a fake thread port name */
		trap->result = 0x2000 + current->pid;
		return 0;

	case 28: /* host_self_trap */
		/* Return host port name (constant in Mach) */
		trap->result = 1;
		return 0;

	default:
		pr_debug("grafted: unhandled mach trap %u\n", trap->trap_number);
		trap->result = -1; /* KERN_FAILURE */
		return 0;
	}
}

static long grafted_ioctl(struct file *file, unsigned int cmd, unsigned long arg)
{
	switch (cmd) {
	case GRAFTED_IOC_PING: {
		__u64 version = GRAFTED_VERSION;
		if (copy_to_user((__u64 __user *)arg, &version, sizeof(version)))
			return -EFAULT;
		return 0;
	}

	case GRAFTED_IOC_REGISTER:
		pr_debug("grafted: process %d registered for Darwin emulation\n",
			 current->pid);
		/* TODO: track registered processes, set up SUD for this task */
		return 0;

	case GRAFTED_IOC_UNREGISTER:
		pr_debug("grafted: process %d unregistered\n", current->pid);
		/* TODO: clean up per-process state */
		return 0;

	case GRAFTED_IOC_MACH_TRAP: {
		struct grafted_mach_trap trap;
		if (copy_from_user(&trap, (void __user *)arg, sizeof(trap)))
			return -EFAULT;

		dispatch_mach_trap(&trap);

		if (copy_to_user((void __user *)arg, &trap, sizeof(trap)))
			return -EFAULT;
		return 0;
	}

	default:
		return -ENOTTY;
	}
}

static const struct file_operations grafted_fops = {
	.owner          = THIS_MODULE,
	.open           = grafted_open,
	.release        = grafted_release,
	.unlocked_ioctl = grafted_ioctl,
};

static struct miscdevice grafted_misc = {
	.minor = MISC_DYNAMIC_MINOR,
	.name  = "grafted",
	.fops  = &grafted_fops,
	.mode  = 0660,  /* owner + group can access - add users to 'grafted' group */
};

int grafted_chardev_init(void)
{
	return misc_register(&grafted_misc);
}

void grafted_chardev_exit(void)
{
	misc_deregister(&grafted_misc);
}
