#![no_std]
#![no_main]
#![feature(never_type)]
// @ivanv: this is only here because we shared ring buffer is not a seperate crate yet
#![feature(core_intrinsics)]

mod shared_ringbuffer;

use sel4cp::{protection_domain, debug_println, Handler, Channel,};
use shared_ringbuffer::{
    ring_init, ring_size, ring_empty, ring_full,
    enqueue_free, enqueue_used, dequeue_free, dequeue_used,
    RingBuffer, RingHandle, BuffDesc
};
use sel4cp::memory_region::{
    declare_memory_region, ReadWrite,
};

const REGION_SIZE: usize = 0x200_000;

const MUX_RX: Channel = Channel::new(0);
const CLIENT: Channel = Channel::new(1);

const BUF_SIZE: usize = 2048;
const NUM_BUFFERS: usize = 512;

const SHARED_DMA_SIZE: usize = BUF_SIZE * NUM_BUFFERS;
const SHARED_DMA_MUX_START: usize = 0x2_400_000;
const SHARED_DMA_CLIENT_START: usize = 0x2_800_000;
const SHARED_DMA_CLIENT_END: usize = SHARED_DMA_CLIENT_START + SHARED_DMA_SIZE;

// @ivanv: fucking stupid
fn void() {}

#[protection_domain]
fn init() -> CopyHandler {
    debug_println!("RUST COPIER init");

    let region_rx_free_mux = unsafe {
        declare_memory_region! {
            <RingBuffer, ReadWrite>(rx_free_mux, REGION_SIZE)
        }
    };
    let region_rx_used_mux = unsafe {
        declare_memory_region! {
            <RingBuffer, ReadWrite>(rx_used_mux, REGION_SIZE)
        }
    };
    let region_rx_free_cli = unsafe {
        declare_memory_region! {
            <RingBuffer, ReadWrite>(rx_free_cli, REGION_SIZE)
        }
    };
    let region_rx_used_cli = unsafe {
        declare_memory_region! {
            <RingBuffer, ReadWrite>(rx_used_cli, REGION_SIZE)
        }
    };

    /* Set up ring buffers shared between the RX multiplexor and the client. */
    let mut rx_ring_mux = ring_init(region_rx_free_mux, region_rx_used_mux, void, true);
    /* For the client shared rings, we are trusting the client will initialise the write_idx and read_idx. */
    let rx_ring_cli = ring_init(region_rx_free_cli, region_rx_used_cli, void, false);

    /* Enqueue free buffers for the mux to access */
    for i in 0..NUM_BUFFERS {
        let addr = SHARED_DMA_MUX_START + (BUF_SIZE * i);
        let res = enqueue_free(&mut rx_ring_mux, addr, BUF_SIZE, 0);
        if res.is_err() {
            debug_println!("COPIER|ERROR: could not enqueue into RX MUX ring: addr is {}, i is: {}", addr, i);
            debug_println!("Reason for enqueue fail: {}", res.unwrap_err());
        }
    }

    CopyHandler {
        rx_ring_mux,
        rx_ring_cli,
        initialised: false
    }
}

struct CopyHandler {
    rx_ring_mux: RingHandle<'static>,
    rx_ring_cli: RingHandle<'static>,
    initialised: bool,
}

fn process_rx_complete(rx_ring_mux: &mut RingHandle, rx_ring_cli: &mut RingHandle) {
    let mux_was_full = ring_full(&rx_ring_mux.used);
    let mux_free_original_size = ring_size(&rx_ring_mux.free);
    let cli_used_was_empty = ring_empty(&rx_ring_cli.used);
    let mut enqueued = 0;
    // We only want to copy buffers if all the dequeues and enqueues will be successful
    while !ring_empty(&rx_ring_mux.used) &&
            !ring_empty(&rx_ring_cli.free) &&
            !ring_full(&rx_ring_mux.free) &&
            !ring_full(&rx_ring_cli.used) {

        // @ivanv: should double check that the error does not happen, rather than unwrapping?
        let BuffDesc { encoded_addr: mux_addr, len: mux_len, cookie: mux_cookie } = dequeue_used(rx_ring_mux).unwrap();
        // get a free one from clients queue.
        let BuffDesc { encoded_addr: client_addr, len: client_len, cookie: client_cookie } = dequeue_free(rx_ring_cli).unwrap();
        if client_addr == 0 || client_addr < SHARED_DMA_CLIENT_START || client_addr >= SHARED_DMA_CLIENT_END {
            debug_println!("COPIER|ERROR: Received an insane address: {}. Address should be between {} and {}",
                client_addr, SHARED_DMA_CLIENT_START, SHARED_DMA_CLIENT_END);
        }

        if client_len < mux_len {
            debug_println!("COPIER|ERROR: Client buffer length is less than MUX buffer length. Client length: {}, MUX length: {}", client_len, mux_len);
        }
        // copy the data over to the clients address space.
        // @ivanv: do the memcpy
        // memcpy((void *)client_addr, (void *)mux_addr, mux_len);

        /* Now that we've copied the data, enqueue the buffer to the client's used ring. */
        let _ = enqueue_used(rx_ring_cli, client_addr, mux_len, client_cookie);
        /* enqueue the old buffer back to dev_rx_ring.free so the driver can use it again. */
        // @ivanv: why don't we use client_len instead of BUF_SIZE?
        let _ = enqueue_free(rx_ring_mux, mux_addr, BUF_SIZE, mux_cookie);

        enqueued += 1;
    }

    if cli_used_was_empty && enqueued > 0 {
        CLIENT.notify_queue();
    }

    /* We only want to signal the mux if the free queue was
        empty and we enqueued something, or that the used queue was full and
        we dequeued something, OR
        potentially the mux pre-empted us (as it's higher prio) and emptied the queue
        while we were enqueuing, and thus the OG size and number of packets we
        processed doesn't add up and the mux could potentially miss this
        empty -> non-empty transition. */
    if (mux_free_original_size == 0 || mux_was_full ||
            mux_free_original_size + enqueued != ring_size(&rx_ring_mux.free))
            && enqueued > 0 {
        if !CLIENT.notify_is_queued() {
            // We need to notify the client, but this should
            // happen first.
            CLIENT.notify();
        }
        MUX_RX.notify_queue();
    }
}

impl Handler for CopyHandler {
    type Error = !;

    fn notified(&mut self, channel: Channel) -> Result<(), Self::Error> {
        if !self.initialised {
            /*
             * Propogate this down the line to ensure everyone is
             * initliased in correct order.
             */
            MUX_RX.notify();
            self.initialised = true;
            return Ok(());
        }

        match channel {
            CLIENT | MUX_RX => process_rx_complete(&mut self.rx_ring_mux, &mut self.rx_ring_cli),
            _ => debug_println!("COPIER|ERROR: unexpected notification from channel: {:?}\n", channel)
        }

        // @ivanv: should probably not always return an error
        Ok(())
    }
}
