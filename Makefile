NAME := stm32f4
ELF := target/thumbv7em-none-eabihf/release/$(NAME)
OBJCOPY_ARGS :=

ifeq ($(shell uname),Linux)
	SUDO := sudo
endif

.PHONY: $(ELF)
$(ELF):
	cargo build --release

$(NAME).bin: $(ELF)
	arm-none-eabi-objcopy -O binary $(OBJCOPY_ARGS) $(ELF) $(NAME).bin
	dfu-suffix -v 0483 -p df11 -a $(NAME).bin

$(NAME).hex: $(ELF)
	arm-none-eabi-objcopy -O ihex $(OBJCOPY_ARGS) $(ELF) $(NAME).hex

.PHONY: test
test:
	cargo test

.PHONY: clean
clean:
	cargo clean

.PHONY: dfu
dfu: $(NAME).bin
	$(SUDO) dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D $(NAME).bin

.DEFAULT_GOAL := $(NAME).bin
