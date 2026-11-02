CC      := riscv64-linux-gnu-gcc
OBJCOPY := riscv64-linux-gnu-objcopy
KERNEL  := ../build/os/kernel
OUT     := ../build/os/examples/hello
include $(KERNEL)/kernel.mk

$(OUT)/Image: $(OUT)/hello.elf
	$(OBJCOPY) -O binary $< $@

$(OUT)/hello.elf: hello.c $(KERNEL)/lib/libkernel.a $(KERNEL)/lib/kernel.ld $(KERNEL)/include/kernel.h
	@mkdir -p $(@D)
	$(CC) $(KERNEL_CFLAGS) -Os -std=gnu11 -Wall -Wextra -o $@ $< $(KERNEL_LDFLAGS) $(KERNEL_LIBS)
