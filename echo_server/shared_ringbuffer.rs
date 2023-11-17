/*
 * Copyright 2023, UNSW
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */
// @ivanv: the buffdesc in the C version is not packed, but it could be

use core::assert;
use core::intrinsics;
use zerocopy::{AsBytes, FromBytes};
use sel4_externally_shared::{ExternallySharedRef};

const RING_SIZE: u32 = 512;

/// Buffer descriptor
#[derive(AsBytes, FromBytes)]
#[repr(C)]
pub struct BuffDesc {
    /// Encoded DMA address
    pub encoded_addr: usize,
    /// Associated memory lengths
    pub len: usize,
    /// Index into client side metadata
    pub cookie: usize,
}

/// Circular buffer containing descriptors
#[derive(AsBytes, FromBytes)]
#[repr(C)]
pub struct RingBuffer {
    write_idx: u32,
    read_idx: u32,
    buffers: [BuffDesc; RING_SIZE as usize],
}

/* Function pointer to be used to 'notify' components on either end of the shared memory */
type Notify = fn();

/* A ring handle for enqueing/dequeuing into  */
pub struct RingHandle<'a> {
    /// Free ring, contains buffers that are empty and available for use.
    pub free: ExternallySharedRef<&'a mut RingBuffer>,
    /// Used ring, contains buffers that are ready for processing.
    pub used: ExternallySharedRef<&'a mut RingBuffer>,
    /// Function to be used to signal that work is queued in the used ring
    notify: Notify,
}

/// Check if the ring buffer is empty
pub fn ring_empty(ring: &ExternallySharedRef<&mut RingBuffer>) -> bool {
    return ring_size(ring) == 0;
}

/// Check if the ring buffer is full
pub fn ring_full(ring: &ExternallySharedRef<&mut RingBuffer>) -> bool {
    ring_size(ring) == RING_SIZE - 1
}

/// Get the number of buffers in the ring
pub fn ring_size(ring: &ExternallySharedRef<&mut RingBuffer>) -> u32 {
    let read_idx = ring.map(|r| & r.read_idx).read();
    let write_idx = ring.map(|r| & r.write_idx).read();
    assert!(write_idx >= read_idx);

    write_idx - read_idx
}

/**
 * Notify the other user of changes to the shared ring buffers.
 *
 * @param ring the ring handle used.
 *
 */
pub fn notify(ring: &RingHandle) {
    return (ring.notify)();
}

/// Enqueue an element to a ring buffer
/// @param ring Ring buffer to enqueue into.
/// @param buffer address into shared memory where data is stored.
/// @param len length of data inside the buffer above.
/// @param cookie optional pointer to data required on dequeueing.
/// @return -1 when ring is empty, 0 on success.
pub fn enqueue(ring: &mut ExternallySharedRef<&mut RingBuffer>, buffer_addr: usize, len: usize, cookie: usize) -> Result<(), &'static str>
{
    assert!(buffer_addr != 0);
    if ring_full(ring) {
        return Err("Trying to enqueue onto a full ring");
    }

    let write_idx = ring.map(|r| &r.write_idx).read();
    let mut buffers = ring.map_mut(|r| &mut r.buffers);
    let mut buffers_slice = buffers.as_mut_slice();
    let mut buffer = buffers_slice.index_mut((write_idx % RING_SIZE) as usize);

    let mut buffer_encoded_addr = buffer.map_mut(|b| &mut b.encoded_addr);
    buffer_encoded_addr.write(buffer_addr);

    let mut buffer_len = buffer.map_mut(|b| &mut b.len);
    buffer_len.write(len);

    let mut buffer_cookie = buffer.map_mut(|b| &mut b.cookie);
    buffer_cookie.write(cookie);

    let mut write_idx_mut = ring.map_mut(|r| &mut r.write_idx);
    unsafe {
        intrinsics::atomic_fence_release();
    }
    write_idx_mut.update(|v| *v += 1);

    Ok(())
}

/**
 * Dequeue an element to a ring buffer.
 *
 * @param ring Ring buffer to dequeue from.
 * @param buffer pointer to the address of where to store buffer address.
 * @param len pointer to variable to store length of data dequeueing.
 * @param cookie pointer optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is empty, 0 on success.
 */
pub fn dequeue(ring: &mut ExternallySharedRef<&mut RingBuffer>) -> Result<BuffDesc, &'static str>
{
    if ring_empty(&ring) {
        return Err("Trying to dequeue from an empty ring");
    }

    let read_idx = ring.map_mut(|r| &mut r.read_idx).read();
    let buffers = ring.map(|r| &r.buffers);
    let buffers_slice = buffers.as_slice();
    let buffer = buffers_slice.index((read_idx % RING_SIZE) as usize);

    let buffer_encoded_addr = buffer.map(|b| &b.encoded_addr);
    let addr = buffer_encoded_addr.read();
    assert!(addr != 0);

    let buffer_len = buffer.map(|b| &b.len);
    let len = buffer_len.read();

    let buffer_cookie = buffer.map(|b| &b.cookie);
    let cookie = buffer_cookie.read();

    let mut read_idx_mut = ring.map_mut(|r| &mut r.read_idx);
    unsafe {
        intrinsics::atomic_fence_release();
    }
    read_idx_mut.update(|v| *v += 1);

    Ok(BuffDesc {
        encoded_addr: addr,
        cookie: cookie,
        len: len
    })
}

/**
 * Enqueue an element into an free ring buffer.
 * This indicates the buffer address parameter is currently free for use.
 *
 * @param ring Ring handle to enqueue into.
 * @param buffer address into shared memory where data is stored.
 * @param len length of data inside the buffer above.
 * @param cookie optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is full, 0 on success.
 */
pub fn enqueue_free(ring_handle: &mut RingHandle, addr: usize, len: usize, cookie: usize) -> Result<(), &'static str> {
    enqueue(&mut ring_handle.free, addr, len, cookie)
}

/**
 * Enqueue an element into a used ring buffer.
 * This indicates the buffer address parameter is currently in use.
 *
 * @param ring Ring handle to enqueue into.
 * @param buffer address into shared memory where data is stored.
 * @param len length of data inside the buffer above.
 * @param cookie optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is full, 0 on success.
 */
pub fn enqueue_used(ring_handle: &mut RingHandle, addr: usize, len: usize, cookie: usize) -> Result<(), &'static str> {
    enqueue(&mut ring_handle.used, addr, len, cookie)
}

/**
 * Dequeue an element from an free ring buffer.
 *
 * @param ring Ring handle to dequeue from.
 * @param buffer pointer to the address of where to store buffer address.
 * @param len pointer to variable to store length of data dequeueing.
 * @param cookie pointer optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is empty, 0 on success.
 */
pub fn dequeue_free(ring: &mut RingHandle) -> Result<BuffDesc, &'static str> {
    dequeue(&mut ring.free)
}

/**
 * Dequeue an element from a used ring buffer.
 *
 * @param ring Ring handle to dequeue from.
 * @param buffer pointer to the address of where to store buffer address.
 * @param len pointer to variable to store length of data dequeueing.
 * @param cookie pointer optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is empty, 0 on success.
 */
pub fn dequeue_used(ring: &mut RingHandle) -> Result<BuffDesc, &'static str> {
    dequeue(&mut ring.used)
}

/**
 * Initialise the shared ring buffer.
 *
 * @param ring ring handle to use.
 * @param free pointer to free ring in shared memory.
 * @param used pointer to 'used' ring in shared memory.
 * @param notify function pointer used to notify the other user.
 * @param buffer_init 1 indicates the read and write indices in shared memory need to be initialised.
 *                    0 inidicates they do not. Only one side of the shared memory regions needs to do this.
 */
pub fn ring_init<'a>(free: ExternallySharedRef<&'a mut RingBuffer>, used: ExternallySharedRef<&'a mut RingBuffer>, notify: Notify, buffer_init: bool) -> RingHandle<'a> {
    let mut ring = RingHandle {
        free: free,
        used: used,
        notify: notify
    };

    if buffer_init {
        ring.free.map_mut(|r| &mut r.write_idx).write(0);
        ring.free.map_mut(|r| &mut r.read_idx).write(0);
        ring.used.map_mut(|r| &mut r.write_idx).write(0);
        ring.used.map_mut(|r| &mut r.read_idx).write(0);
    }

    ring
}
