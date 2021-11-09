//! The root task.

use drone_cortexm::{fib, reg::prelude::*, thr::prelude::*};
use drone_stm32_map::periph::gpio::pin::{GpioPinMap, GpioPinPeriph};
use drone_stm32_map::periph::gpio::{periph_gpio_k3, periph_gpio_k_head};
use drone_stm32_map::periph::sys_tick::{periph_sys_tick, SysTickPeriph};
use drone_stm32_map::reg;
use drone_cortexm::map::thr::*;
use futures::prelude::*;

use crate::{thread, thread::ThrsInit, thread::Rcc, thread::SysTick, Regs};

const SYSCLK: u32 = 168_000_000;
const HPRE: u32 = 1; // = SYSCLK
const PPRE1: u32 = 0b101; // SYSCLK / 4 = 42MHz
const PPRE2: u32 = 0b100; // SYSCLK / 2 = 84MHz
const PLL_SELECTED: u32 = 0b10;
const FLASH_LATENCY: u32 = (SYSCLK - 1) / 30_000_000;

#[derive(Debug)]
pub struct TickOverflow;

type RccRegs = (
    reg::rcc::Cfgr<Srt>,
    reg::rcc::Cir<Srt>,
    reg::rcc::Cr<Srt>,
    reg::rcc::Pllcfgr<Srt>,
    reg::flash::Acr<Srt>,
);

async fn setup(rcc: Rcc, regs: RccRegs) {
    let (cfgr, cir, cr, pllcfgr, flash_acr) = regs;

    rcc.enable_int();
    cir.modify(|r| r.set_hserdyie().set_pllrdyie());

    let reg::rcc::Cir { hserdyc, hserdyf, .. } = cir;

    let hse_ready = rcc.add_future(fib::new_fn(move || {
        if !hserdyf.read_bit() {
            return fib::Yielded(());
        }
        hserdyc.set_bit();
        fib::Complete(())
    }));
    cr.modify(|r| r.set_hseon());
    hse_ready.await;

    flash_acr.modify(|r| r.write_latency(FLASH_LATENCY));

    // PLL = (8MHz / M) * N / P = (8MHz / 8) * 336 / 2 = 168MHz
    pllcfgr.modify(|r| r.write_pllm(8).write_plln(336).write_pllp(0).write_pllq(7).set_pllsrc());
    cr.modify(|r| r.set_pllon());
    let reg::rcc::Cir { pllrdyc, pllrdyf, .. } = cir;
    let pll_ready = rcc.add_future(fib::new_fn(move || {
        if !pllrdyf.read_bit() {
            return fib::Yielded(());
        }
        pllrdyc.set_bit();
        fib::Complete(())
    }));
    pll_ready.await;

    cfgr.modify(|r| r.write_hpre(HPRE).write_ppre1(PPRE1).write_ppre2(PPRE2));
    cfgr.modify(|r| r.write_sw(PLL_SELECTED));
}

async fn beacon<T: GpioPinMap>(
    pin: GpioPinPeriph<T>,
    systick: SysTickPeriph,
    thread_systick: SysTick,
) -> Result<(), TickOverflow> {
    let fiber = fib::new_fn(|| fib::Yielded(Some(1)));
    let mut tick_stream = thread_systick.add_pulse_try_stream(|| Err(TickOverflow), fiber);
    systick.stk_val.store(|r| r.write_current(0));
    systick.stk_load.store(|r| r.write_reload(SYSCLK / 8));
    systick.stk_ctrl.store(|r| r.set_tickint().set_enable());

    let mut counter = 0;
    while let Some(tick) = tick_stream.next().await {
        for _ in 0..tick?.get() {
            if counter == 0 {
                println!("sec");
            }
            match counter {
                0 | 2 => pin.gpio_bsrr_br.set_bit(),
                _ => pin.gpio_bsrr_bs.set_bit(),
            }
            counter = (counter + 1) % 8;
        }
    }

    Ok(())
}

/// The root task handler.
#[inline(never)]
pub fn handler(reg: Regs, thr_init: ThrsInit) {
    let thread = thread::init(thr_init);

    thread.hard_fault.add_once(|| panic!("Hard Fault"));

    reg.flash_acr.modify(|r| r.write_latency(2));
    let regs = (reg.rcc_cfgr, reg.rcc_cir, reg.rcc_cr, reg.rcc_pllcfgr, reg.flash_acr);
    setup(thread.rcc, regs).root_wait();

    reg.rcc_apb1enr.pwren.set_bit();

    let gpio_k = periph_gpio_k_head!(reg);
    gpio_k.rcc_busenr_gpioen.set_bit();
    let gpio_k3 = periph_gpio_k3!(reg);
    gpio_k3.gpio_moder_moder.write_bits(0b01);
    gpio_k3.gpio_bsrr_br.set_bit();

    let sys_tick = periph_sys_tick!(reg);
    beacon(gpio_k3, sys_tick, thread.sys_tick).root_wait().expect("beacon fail");

    // Enter a sleep state on ISR exit.
    reg.scb_scr.sleeponexit.set_bit();
}
