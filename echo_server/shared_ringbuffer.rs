/*
 * Copyright 2023, UNSW
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// @ivanv: figure out cookie pointer
// @ivanv: make sure to add thread memory release barrier

use core::assert;
// use core::ffi::c_void;
use zerocopy::{AsBytes, FromBytes};
use sel4cp::memory_region::{Volatile};

const RING_SIZE: u32 = 512;

/* Buffer descriptor */
#[derive(AsBytes, FromBytes)]
#[repr(C)]
pub struct BuffDesc {
    encoded_addr: usize, /* encoded dma addresses */
    cookie: usize, /* index into client side metadata */
    len: usize,            /* associated memory lengths */ // @ivanv changed from u32 to usize to not have to pack struct
}

/* Circular buffer containing descriptors */
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
    pub free: Volatile<&'a mut RingBuffer>,
    pub used: Volatile<&'a mut RingBuffer>,
    /* Function to be used to signal that work is queued in the used ring */
    notify: Notify,
}

/**
 * Check if the ring buffer is empty.
 *
 * @param ring ring buffer to check.
 *
 * @return true indicates the buffer is empty, false otherwise.
 */
pub fn ring_empty(ring: &Volatile<&mut RingBuffer>) -> bool {
    return ring_size(ring) == 0;
}

/**
 * Check if the ring buffer is full
 *
 * @param ring ring buffer to check.
 *
 * @return true indicates the buffer is full, false otherwise.
 */
pub fn ring_full(ring: &Volatile<&mut RingBuffer>) -> bool
{
    return ring_size(ring) == RING_SIZE - 1;
}

pub fn ring_size(ring: &Volatile<&mut RingBuffer>) -> u32
{
    let read_idx = ring.map(|r| & r.read_idx);
    let write_idx = ring.map(|r| & r.write_idx);
    assert!(write_idx.read() - read_idx.read() >= 0);
    return write_idx.read() - read_idx.read();
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

/**
 * Enqueue an element to a ring buffer
 *
 * @param ring Ring buffer to enqueue into.
 * @param buffer address into shared memory where data is stored.
 * @param len length of data inside the buffer above.
 * @param cookie optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is empty, 0 on success.
 */
pub fn enqueue(ring: &mut Volatile<&mut RingBuffer>, buffer_addr: usize, len: usize, cookie: usize) -> Result<(), &'static str>
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

    // THREAD_MEMORY_RELEASE();
    let mut write_idx_mut = ring.map_mut(|r| &mut r.write_idx);
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
pub fn dequeue(ring: &mut Volatile<&mut RingBuffer>, addr: &mut usize, len: &mut usize, cookie: &mut usize) -> Result<(), &'static str>
{
    if ring_empty(&ring) {
        return Err("Trying to dequeue from an empty ring");
    }

    let read_idx = ring.map_mut(|r| &mut r.read_idx).read();
    let buffers = ring.map(|r| &r.buffers);
    let buffers_slice = buffers.as_slice();
    let buffer = buffers_slice.index((read_idx % RING_SIZE) as usize);

    // assert!((*ring).buffers[idx].encoded_addr != 0);

    let buffer_encoded_addr = buffer.map(|b| &b.encoded_addr);
    *addr = buffer_encoded_addr.read();

    let buffer_len = buffer.map(|b| &b.len);
    *len = buffer_len.read();

    let buffer_cookie = buffer.map(|b| &b.cookie);
    *cookie = buffer_cookie.read();

    // THREAD_MEMORY_RELEASE();
    let mut read_idx_mut = ring.map_mut(|r| &mut r.read_idx);
    read_idx_mut.update(|v| *v += 1);

    Ok(())
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
    return enqueue(&mut ring_handle.free, addr, len, cookie);
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
    return enqueue(&mut ring_handle.used, addr, len, cookie);
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
pub fn dequeue_free(ring: &mut RingHandle, addr: &mut usize, len: &mut usize, cookie: &mut usize) -> Result<(), &'static str> {
    return dequeue(&mut ring.free, addr, len, cookie);
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
pub fn dequeue_used(ring: &mut RingHandle, addr: &mut usize, len: &mut usize, cookie: &mut usize) -> Result<(), &'static str> {
    return dequeue(&mut ring.used, addr, len, cookie);
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
pub fn ring_init<'a>(free: Volatile<&'a mut RingBuffer>, used: Volatile<&'a mut RingBuffer>, notify: Notify, buffer_init: bool) -> RingHandle<'a> {
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
