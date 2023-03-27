use alloc::sync::Arc;
use core::{
    arch::asm,
    future::Future,
    mem::transmute,
    pin::Pin,
    task::{Context, Poll},
};

use log::error;
use riscv::register::{
    scause,
    scause::{Exception, Interrupt},
    stval,
};

use crate::{
    constant::TRAMPOLINE,
    executor,
    executor::TrapContext,
    syscall::SystemCall,
    task::Thread,
    timer,
};

/// The `ControlFlow` enum specifies the operation that the executor should execute on a thread
/// prior to returning to user space.
pub enum ControlFlow {
    Continue,
    Yield,
    Break,
}

/// The `thread_loop` future represents the lifetime of a user thread.
async fn thread_loop(thread: Arc<Thread>) {
    // There are two mappings of the _enter_user_space function in the kernel page table.
    // The first mapping is included in the identical mapping for all physical addresses,
    // While the second mapping is included in the `TRAMPOLINE` page.
    // When invoking the `_enter_user_space` function, the second mapping should be used
    // because it is also mapped in the user space, ensuring that the CPU can continue executing
    // at the same address after switching to the user space page table.
    // The function pointer is cast to the `fn(&mut TrapContext, usize)` type to ensure proper
    // calling convention.
    let _enter_user_space: fn(&mut TrapContext, usize) = {
        unsafe { transmute(_enter_user_space as usize - _enter_kernel_space as usize + TRAMPOLINE) }
    };

    loop {
        let trap_context = thread.state().user_trap_context_mut();
        _enter_user_space(trap_context, thread.satp());

        let scause = scause::read();
        let stval = stval::read();
        let task_action = match scause.cause() {
            scause::Trap::Exception(Exception::UserEnvCall) => {
                SystemCall::new(&thread).execute().await
            }
            scause::Trap::Exception(Exception::StoreFault)
            | scause::Trap::Exception(Exception::StorePageFault)
            | scause::Trap::Exception(Exception::LoadFault)
            | scause::Trap::Exception(Exception::LoadPageFault) => {
                error!("page fault at {:#x}", stval);
                ControlFlow::Break
            }
            scause::Trap::Exception(Exception::IllegalInstruction) => {
                error!("illegal instruction");
                ControlFlow::Break
            }
            scause::Trap::Exception(Exception::InstructionMisaligned) => {
                error!("misaligned instruction");
                ControlFlow::Break
            }
            scause::Trap::Interrupt(Interrupt::SupervisorTimer) => {
                timer::set_trigger();
                ControlFlow::Yield
            }
            _ => {
                panic!("unsupported trap {:?}", scause.cause())
            }
        };

        match task_action {
            ControlFlow::Continue => continue,
            ControlFlow::Yield => yield_now().await,
            ControlFlow::Break => break,
        }
    }
}

pub fn spawn_thread(thread: Arc<Thread>) {
    let (runnable, task) = executor::spawn(thread_loop(thread));
    runnable.schedule();
    task.detach();
}

#[naked]
#[link_section = ".text.trampoline"]
unsafe extern "C" fn _enter_kernel_space() {
    asm!(
        ".p2align 2",
        // Read the address of `trap_context` from sscratch
        // and store the user stack pointer to sscratch
        "csrrw sp, sscratch, sp",
        // Store the registers to `trap_context.user_register`
        "sd zero, 0 * 8(sp)",
        "sd ra, 1 * 8(sp)",
        "sd gp, 3 * 8(sp)",
        "sd tp, 4 * 8(sp)",
        "sd t0, 5 * 8(sp)",
        "sd t1, 6 * 8(sp)",
        "sd t2, 7 * 8(sp)",
        "sd s0, 8 * 8(sp)",
        "sd s1, 9 * 8(sp)",
        "sd a0, 10 * 8(sp)",
        "sd a1, 11 * 8(sp)",
        "sd a2, 12 * 8(sp)",
        "sd a3, 13 * 8(sp)",
        "sd a4, 14 * 8(sp)",
        "sd a5, 15 * 8(sp)",
        "sd a6, 16 * 8(sp)",
        "sd a7, 17 * 8(sp)",
        "sd s2, 18 * 8(sp)",
        "sd s3, 19 * 8(sp)",
        "sd s4, 20 * 8(sp)",
        "sd s5, 21 * 8(sp)",
        "sd s6, 22 * 8(sp)",
        "sd s7, 23 * 8(sp)",
        "sd s8, 24 * 8(sp)",
        "sd s9, 25 * 8(sp)",
        "sd s10, 26 * 8(sp)",
        "sd s11, 27 * 8(sp)",
        "sd t3, 28 * 8(sp)",
        "sd t4, 29 * 8(sp)",
        "sd t5, 30 * 8(sp)",
        "sd t6, 31 * 8(sp)",
        // Save sstatus to `trap_context.user_sstatus`
        "csrr t0, sstatus",
        "sd t0, 32 * 8(sp)",
        // Save sepc to `trap_context.user_sepc`
        "csrr t1, sepc",
        "sd  t1, 33 * 8(sp)",
        // Store the address of `trap_context` to sscratch
        // and read the user stack pointer to t2
        "csrrw t2, sscratch, sp",
        // Store the user stack pointer to `trap_context.user_register`
        "sd t2, 2 * 8(sp)",
        // Read `trap_context.kernel_satp` to t3
        "ld t3, 35 * 8(sp)",
        // Read the stack pointer from `trap_context.kernel_stack`
        "ld sp, 34 * 8(sp)",
        // Write the address of the page table of the kernel to satp
        "csrw satp, t3",
        "sfence.vma",
        // Read the return address, global pointer, thread pointer from the kernel stack
        "ld ra, 0 * 8(sp)",
        "ld gp, 1 * 8(sp)",
        "ld tp, 2 * 8(sp)",
        // Store the callee-saved registers on the kernel stack
        "ld s0, 3 * 8(sp)",
        "ld s1, 4 * 8(sp)",
        "ld s2, 5 * 8(sp)",
        "ld s3, 6 * 8(sp)",
        "ld s4, 7 * 8(sp)",
        "ld s5, 8 * 8(sp)",
        "ld s6, 9 * 8(sp)",
        "ld s7, 10 * 8(sp)",
        "ld s8, 11 * 8(sp)",
        "ld s9, 12 * 8(sp)",
        "ld s10, 13 * 8(sp)",
        "ld s11, 14 * 8(sp)",
        // deallocate 15 words on the kernel stack
        "addi sp, sp, 15 * 8",
        "jr ra",
        options(noreturn)
    )
}

#[naked]
#[link_section = ".text.trampoline"]
unsafe extern "C" fn _enter_user_space(trap_context: &mut TrapContext, user_satp: usize) {
    asm!(
        ".p2align 2",
        // allocate 15 words on the kernel stack
        "addi sp, sp, -15 * 8",
        // Store the return address, global pointer, thread pointer on the kernel stack
        "sd ra, 0 * 8(sp)",
        "sd gp, 1 * 8(sp)",
        "sd tp, 2 * 8(sp)",
        // Store the callee-saved registers on the kernel stack
        "sd s0, 3 * 8(sp)",
        "sd s1, 4 * 8(sp)",
        "sd s2, 5 * 8(sp)",
        "sd s3, 6 * 8(sp)",
        "sd s4, 7 * 8(sp)",
        "sd s5, 8 * 8(sp)",
        "sd s6, 9 * 8(sp)",
        "sd s7, 10 * 8(sp)",
        "sd s8, 11 * 8(sp)",
        "sd s9, 12 * 8(sp)",
        "sd s10, 13 * 8(sp)",
        "sd s11, 14 * 8(sp)",
        // Write the address of the page table of the process to satp
        // and read the address of the page table of the kernel to a1
        "csrrw a1, satp, a1",
        "sfence.vma",
        // Store the stack pointer to `trap_context.kernel_stack`
        // and move the stack pointer to `trap_context`
        "sd sp, 34 * 8(a0)",
        "mv sp, a0",
        // Store the address of the page table of the kernel to `trap_context.kernel_satp`
        "sd a1, 35 * 8(sp)",
        // Read `trap_context.user_sstatus` to t0
        "ld t0, 32 * 8(sp)",
        "csrw sstatus, t0",
        // Read `trap_context.user_sepc` to t1
        "ld t1, 33 * 8(sp)",
        "csrw sepc, t1",
        // Read the registers from `trap_context.user_register`
        "ld zero, 0 * 8(sp)",
        "ld ra, 1 * 8(sp)",
        "ld gp, 3 * 8(sp)",
        "ld tp, 4 * 8(sp)",
        "ld t0, 5 * 8(sp)",
        "ld t1, 6 * 8(sp)",
        "ld t2, 7 * 8(sp)",
        "ld s0, 8 * 8(sp)",
        "ld s1, 9 * 8(sp)",
        "ld a0, 10 * 8(sp)",
        "ld a1, 11 * 8(sp)",
        "ld a2, 12 * 8(sp)",
        "ld a3, 13 * 8(sp)",
        "ld a4, 14 * 8(sp)",
        "ld a5, 15 * 8(sp)",
        "ld a6, 16 * 8(sp)",
        "ld a7, 17 * 8(sp)",
        "ld s2, 18 * 8(sp)",
        "ld s3, 19 * 8(sp)",
        "ld s4, 20 * 8(sp)",
        "ld s5, 21 * 8(sp)",
        "ld s6, 22 * 8(sp)",
        "ld s7, 23 * 8(sp)",
        "ld s8, 24 * 8(sp)",
        "ld s9, 25 * 8(sp)",
        "ld s10, 26 * 8(sp)",
        "ld s11, 27 * 8(sp)",
        "ld t3, 28 * 8(sp)",
        "ld t4, 29 * 8(sp)",
        "ld t5, 30 * 8(sp)",
        "ld t6, 31 * 8(sp)",
        // Save the address of `trap_context` to sscratch
        "csrw sscratch, sp",
        // Read the user stack pointer from `trap_context.user_register`
        "ld sp, 2 * 8(sp)",
        "sret",
        options(noreturn)
    )
}

async fn yield_now() {
    YieldFuture::new().await
}

struct YieldFuture {
    state: bool,
}

impl YieldFuture {
    fn new() -> Self {
        YieldFuture { state: false }
    }
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        if self.state {
            return Poll::Ready(());
        }
        self.state = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
