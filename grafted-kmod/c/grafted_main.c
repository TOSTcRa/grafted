// SPDX-License-Identifier: GPL-2.0
/*
 * grafted_main.c — Grafted kernel module entry point
 *
 * Registers the module and initializes the /dev/grafted char device
 * for userspace communication.
 */

#include <linux/init.h>
#include <linux/module.h>
#include <linux/kernel.h>

#include "grafted_chardev.h"

static int __init grafted_init(void)
{
	int ret;

	pr_info("grafted: loading Darwin compatibility module v0.1.0\n");

	ret = grafted_chardev_init();
	if (ret) {
		pr_err("grafted: failed to initialize char device: %d\n", ret);
		return ret;
	}

	pr_info("grafted: module loaded, /dev/grafted ready\n");
	return 0;
}

static void __exit grafted_exit(void)
{
	grafted_chardev_exit();
	pr_info("grafted: module unloaded\n");
}

module_init(grafted_init);
module_exit(grafted_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Grafted Contributors");
MODULE_DESCRIPTION("Darwin compatibility layer — Mach trap and syscall translation");
MODULE_VERSION("0.1.0");
