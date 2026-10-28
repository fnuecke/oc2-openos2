# Runs inside the container from Dockerfile; see build.gradle.kts. Output goes into build/os.

CROSS_COMPILE ?= riscv64-linux-gnu-
MUSL          ?= /opt/musl
CC            := $(CROSS_COMPILE)gcc-12
OBJCOPY       := $(CROSS_COMPILE)objcopy

SRC    := src/main/c
DISK   := src/main/disk
LUA    := external/lua
CJSON  := external/lua-cjson
TLSF   := external/tlsf
BUILD  := build/os
# Written by Gradle when the toolchain container changes, which then rebuilds everything.
TOOLCHAIN := $(wildcard build/toolchain-image.id)

# The disk image is a small oc2 hard drive, see README.md.
DISK_SIZE ?= 2M

ARCH_FLAGS    := -march=rv64imafdc_zicsr_zifencei -mabi=lp64d -mcmodel=medlow
INCLUDES      := -nostdinc -isystem $(MUSL)/include -isystem $(shell $(CC) -print-file-name=include)
CFLAGS        := $(ARCH_FLAGS) $(INCLUDES) -Os -g -fno-pie -fno-stack-protector -fno-asynchronous-unwind-tables \
                 -ffunction-sections -fdata-sections -ffile-prefix-map=$(CURDIR)=. -MMD -MP
# The kernel proper only assumes app_main (see kernel.h). The application on top of it,
# Lua and libs in our case (src/main/c/lua), is the only part that sees Lua's headers.
KERNEL_CFLAGS := $(CFLAGS) -std=gnu11 -Wall -Wextra -I$(SRC) -I$(TLSF)
APP_CFLAGS    := $(CFLAGS) -std=gnu11 -Wall -Wextra -I$(SRC) -I$(LUA) -include $(SRC)/lua/lua_conf.h
LUA_CFLAGS    := $(CFLAGS) -std=gnu99 -Wall -include $(SRC)/lua/lua_conf.h
LDFLAGS       := $(ARCH_FLAGS) -static -nostdlib -no-pie -Wl,--gc-sections -Wl,--build-id=none \
                 -Wl,-Map,$(BUILD)/kernel.map -T $(SRC)/kernel.ld
LIBS          := $(MUSL)/lib/libc.a -lgcc

KERNEL_SRCS := start.S boot.c main.c trap.c time.c power.c alloc.c libc.c fdt.c \
	drivers/console.c drivers/serial.c drivers/plic.c drivers/rtc.c drivers/syscon.c \
	drivers/virtio/virtio.c drivers/virtio/vblk.c drivers/virtio/vcon.c \
	fs/vfs.c fs/devfs.c fs/ext2.c fs/mkfs.c fs/p9.c fs/sysfs.c
APP_SRCS := luaapi.c lposix.c signals.c stdio.c
LUA_SRCS := lapi.c lcode.c lctype.c ldebug.c ldo.c ldump.c lfunc.c lgc.c llex.c lmem.c lobject.c \
	lopcodes.c lparser.c lstate.c lstring.c ltable.c ltm.c lundump.c lvm.c lzio.c \
	lauxlib.c lbaselib.c lcorolib.c ldblib.c lmathlib.c loadlib.c lstrlib.c ltablib.c lutf8lib.c
CJSON_SRCS := lua_cjson.c strbuf.c fpconv.c
# cjson's error paths print to stderr, which would need musl's FILE; lua/stdio.c has console versions.
CJSON_DEFINES := -Dfprintf=app_fprintf -Dvfprintf=app_vfprintf -Dexit=app_exit

KERNEL_OBJS := $(patsubst %,$(BUILD)/kernel/%.o,$(KERNEL_SRCS)) $(BUILD)/tlsf/tlsf.o
APP_OBJS    := $(patsubst %.c,$(BUILD)/app/%.o,$(APP_SRCS))
LUA_OBJS    := $(patsubst %.c,$(BUILD)/lua/%.o,$(LUA_SRCS))
CJSON_OBJS  := $(patsubst %.c,$(BUILD)/cjson/%.o,$(CJSON_SRCS))
DEPS        := Makefile $(TOOLCHAIN)

# The disk image is always rebuilt: it's quick, and this way we pick up file deletions in userland.
.PHONY: all clean $(BUILD)/disk.img
all: $(BUILD)/Image $(BUILD)/disk.img

$(BUILD)/Image: $(BUILD)/kernel.elf
	$(OBJCOPY) -O binary $< $@

$(BUILD)/kernel.elf: $(KERNEL_OBJS) $(APP_OBJS) $(LUA_OBJS) $(CJSON_OBJS) $(SRC)/kernel.ld $(DEPS)
	$(CC) $(LDFLAGS) -o $@ $(KERNEL_OBJS) $(APP_OBJS) $(LUA_OBJS) $(CJSON_OBJS) $(LIBS)

$(BUILD)/kernel/%.c.o: $(SRC)/%.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(KERNEL_CFLAGS) -c -o $@ $<

# Runs before paging, see boot.c.
$(BUILD)/kernel/boot.c.o: $(SRC)/boot.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(KERNEL_CFLAGS) -mcmodel=medany -mno-relax -c -o $@ $<

$(BUILD)/kernel/%.S.o: $(SRC)/%.S $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(ARCH_FLAGS) -c -o $@ $<

$(BUILD)/tlsf/%.o: $(TLSF)/%.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -std=gnu11 -DNDEBUG -c -o $@ $<

$(BUILD)/app/%.o: $(SRC)/lua/%.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(APP_CFLAGS) -c -o $@ $<

$(BUILD)/lua/%.o: $(LUA)/%.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(LUA_CFLAGS) -c -o $@ $<

$(BUILD)/cjson/%.o: $(CJSON)/%.c $(DEPS)
	@mkdir -p $(dir $@)
	$(CC) $(LUA_CFLAGS) $(CJSON_DEFINES) -I$(LUA) -c -o $@ $<

$(BUILD)/disk.img: $(BUILD)/Image
	SIZE=$(DISK_SIZE) ./mkdisk.sh $@ $(BUILD)/Image $(DISK)

clean:
	rm -rf $(BUILD)

# Header dependencies, written by the compiler (-MMD).
-include $(patsubst %.o,%.d,$(KERNEL_OBJS) $(APP_OBJS) $(LUA_OBJS) $(CJSON_OBJS))
