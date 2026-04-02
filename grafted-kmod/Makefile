# Grafted kernel module build system
#
# Prerequisites:
#   sudo dnf install kernel-devel
#   rustup install 1.90.0  (must match CONFIG_RUSTC_VERSION in kernel config)
#
# Build:
#   make
#   sudo insmod grafted.ko
#   sudo rmmod grafted

KDIR ?= /lib/modules/$(shell uname -r)/build
PWD := $(shell pwd)

obj-m := grafted.o
grafted-y := c/grafted_main.o c/grafted_chardev.o

# Rust source (compiled by Kbuild's Rust support)
# grafted-y += src/mach_trap.o  # TODO: enable when Rust Kbuild integration is ready

.PHONY: all clean load unload

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

load:
	sudo insmod grafted.ko

unload:
	sudo rmmod grafted

# Create the device node if it doesn't exist via udev
# The module registers a misc device, so udev handles /dev/grafted automatically.
