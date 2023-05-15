#![no_std]
#![no_main]
#![feature(never_type)]

mod shared_ringbuffer;

use sel4cp::{debug_println, main, Handler, Channel};
use shared_ringbuffer::{ring_init, enqueue_free, RingBuffer, RingHandle};
use sel4cp::memory_region::{
    declare_memory_region, MemoryRegion, ReadWrite,
};

const REGION_SIZE: usize = 0x200_000;

const MUX_RX: Channel = Channel::new(0);
const CLIENT: Channel = Channel::new(1);

const BUF_SIZE: usize = 2048;
const NUM_BUFFERS: usize = 512;
const SHARED_DMA_SIZE: usize = BUF_SIZE * NUM_BUFFERS;

static mut initialised: bool = false;

const SHARED_DMA_MUX_START: usize = 0x2_400_000;
const SHARED_DMA_CLIENT_START: usize = 0x2_800_000;

// @ivanv: fucking stupid
fn void() {}

#[main]
fn main() -> CopyHandler {
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
    let mut rx_ring_cli = ring_init(region_rx_free_cli, region_rx_used_cli, void, false);

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
        rx_ring_cli
    }
}

struct CopyHandler {
    rx_ring_mux: RingHandle<'static>,
    rx_ring_cli: RingHandle<'static>,
}

impl Handler for CopyHandler {
    type Error = !;

    fn notified(&mut self, _: Channel) -> Result<(), Self::Error> {
        Ok(())
    }
}

// static rx_ring_mux: RingHandle = {};
// static rx_ring_cli: RingHandle = {};

// void process_rx_complete(void)
// {
//     bool mux_was_full = ring_full(rx_ring_mux.used_ring);
//     uint64_t mux_free_original_size = ring_size(rx_ring_mux.free_ring);
//     bool cli_used_was_empty = ring_empty(rx_ring_cli.used_ring);
//     uint64_t enqueued = 0;
//     // We only want to copy buffers if all the dequeues and enqueues will be successful
//     while (!ring_empty(rx_ring_mux.used_ring) &&
//             !ring_empty(rx_ring_cli.free_ring) &&
//             !ring_full(rx_ring_mux.free_ring) &&
//             !ring_full(rx_ring_cli.used_ring)) {
//         uintptr_t m_addr, c_addr = 0;
//         unsigned int m_len, c_len = 0;
//         void *cookie = NULL;
//         void *cookie2 = NULL;
//         int err;

//         err = dequeue_used(&rx_ring_mux, &m_addr, &m_len, &cookie);
//         assert(!err);
//         // get a free one from clients queue.
//         err = dequeue_free(&rx_ring_cli, &c_addr, &c_len, &cookie2);
//         assert(!err);
//         if (!c_addr ||
//                 c_addr < shared_dma_vaddr_cli ||
//                 c_addr >= shared_dma_vaddr_cli + SHARED_DMA_SIZE)
//         {
//             print("COPY|ERROR: Received an insane address: ");
//             puthex64(c_addr);
//             print(". Address should be between ");
//             puthex64(shared_dma_vaddr_cli);
//             print(" and ");
//             puthex64(shared_dma_vaddr_cli + SHARED_DMA_SIZE);
//             print("\n");
//         }

//         if (c_len < m_len) {
//             print("COPY|ERROR: client buffer length is less than mux buffer length.\n");
//             print("client length: ");
//             puthex64(c_len);
//             print(" mux length: ");
//             puthex64(m_len);
//             print("\n");
//         }
//         // copy the data over to the clients address space.
//         memcpy((void *)c_addr, (void *)m_addr, m_len);

//         /* Now that we've copied the data, enqueue the buffer to the client's used ring. */
//         err = enqueue_used(&rx_ring_cli, c_addr, m_len, cookie2);
//         assert(!err);
//         /* enqueue the old buffer back to dev_rx_ring.free so the driver can use it again. */
//         err = enqueue_free(&rx_ring_mux, m_addr, BUF_SIZE, cookie);
//         assert(!err);

//         enqueued += 1;
//     }

//     if (cli_used_was_empty && enqueued) {
//         sel4cp_notify_delayed(CLIENT_CH);
//     }

//     /* We only want to signal the mux if the free queue was
//         empty and we enqueued something, or that the used queue was full and 
//         we dequeued something, OR 
//         potentially the mux pre-empted us (as it's higher prio) and emptied the queue
//         while we were enqueuing, and thus the OG size and number of packets we 
//         processed doesn't add up and the mux could potentially miss this 
//         empty -> non-empty transition. */
//     if ((mux_free_original_size == 0 || mux_was_full || 
//             mux_free_original_size + enqueued != ring_size(rx_ring_mux.free_ring)) 
//             && enqueued) {
//         if (have_signal) {
//             // We need to notify the client, but this should
//             // happen first. 
//             sel4cp_notify(CLIENT_CH);
//         }
//         sel4cp_notify_delayed(MUX_RX_CH);
//     }
// }

// void notified(sel4cp_channel ch)
// {
//     if (!initialised) {
//         /*
//          * Propogate this down the line to ensure everyone is
//          * initliased in correct order.
//          */
//         sel4cp_notify(MUX_RX_CH);
//         initialised = 1;
//         return;
//     }

//     if (ch == CLIENT_CH || ch == MUX_RX_CH) {
//         /* We have one job. */
//         process_rx_complete();
//     } else {
//         print("COPY|ERROR: unexpected notification from channel: ");
//         puthex64(ch);
//         print("\n");
//         assert(0);
//     }
// }

