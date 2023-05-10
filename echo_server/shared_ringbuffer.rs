/*
 * Copyright 2023, UNSW
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// @ivanv: figure out cookie pointer
// @ivanv: make sure to add thread memory release barrier

use core::assert;
use core::ffi::c_void;

const RING_SIZE: u32 = 512;

/* Buffer descriptor */
pub struct BuffDesc {
    encoded_addr: usize, /* encoded dma addresses */
    len: u32,            /* associated memory lengths */
    cookie: *mut c_void, /* index into client side metadata */
}

/* Circular buffer containing descriptors */
pub struct RingBuffer {
    write_idx: u32,
    read_idx: u32,
    buffers: [BuffDesc; RING_SIZE as usize],
}

/* Function pointer to be used to 'notify' components on either end of the shared memory */
type Notify = fn();

/* A ring handle for enqueing/dequeuing into  */
pub struct RingHandle {
    free_ring: *mut RingBuffer,
    used_ring: *mut RingBuffer,
    /* Function to be used to signal that work is queued in the used_ring */
    notify: Notify,
}

/**
 * Check if the ring buffer is empty.
 *
 * @param ring ring buffer to check.
 *
 * @return true indicates the buffer is empty, false otherwise.
 */
pub fn ring_empty(ring: *mut RingBuffer) -> bool {
    unsafe {
        return ring_size(ring) == 0;
    }
}

/**
 * Check if the ring buffer is full
 *
 * @param ring ring buffer to check.
 *
 * @return true indicates the buffer is full, false otherwise.
 */
pub fn ring_full(ring: *mut RingBuffer) -> bool
{
    unsafe {
        return ring_size(ring) == RING_SIZE - 1;
    }
}

pub fn ring_size(ring: *mut RingBuffer) -> u32
{
    unsafe {
        assert!((*ring).write_idx - (*ring).read_idx >= 0);
        return (*ring).write_idx - (*ring).read_idx;
    }
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
pub fn enqueue(ring: *mut RingBuffer, buffer: usize, len: u32, cookie: *mut c_void) -> Result<(), &'static str>
{
    assert!(buffer != 0);
    unsafe {
        if ring_full(ring) {
            return Err("Trying to enqueue onto a full ring");
        }

        let idx = ((*ring).write_idx % RING_SIZE) as usize;
        (*ring).buffers[idx].encoded_addr = buffer;
        (*ring).buffers[idx].len = len;
        (*ring).buffers[idx].cookie = cookie;

        // THREAD_MEMORY_RELEASE();
        (*ring).write_idx += 1;
    }

    Ok(())
}

/**
 * Dequeue an element to a ring buffer.
 *
 * @param ring Ring buffer to Dequeue from.
 * @param buffer pointer to the address of where to store buffer address.
 * @param len pointer to variable to store length of data dequeueing.
 * @param cookie pointer optional pointer to data required on dequeueing.
 *
 * @return -1 when ring is empty, 0 on success.
 */
pub fn dequeue(ring: *mut RingBuffer, addr: &mut usize, len: &mut u32, cookie: *mut *mut c_void) -> Result<(), &'static str>
{
    unsafe {
        if ring_empty(ring) {
            return Err("Trying to dequeue from an empty ring");
        }

        let idx = ((*ring).read_idx % RING_SIZE) as usize;
        assert!((*ring).buffers[idx].encoded_addr != 0);
        *addr = (*ring).buffers[idx].encoded_addr;
        *len = (*ring).buffers[idx].len;
        *cookie = (*ring).buffers[idx].cookie;

        // THREAD_MEMORY_RELEASE();
        (*ring).read_idx += 1;
    }

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
pub fn enqueue_free(ring: &mut RingHandle, addr: usize, len: u32, cookie: *mut c_void) -> Result<(), &'static str>
{
    return enqueue(ring.free_ring, addr, len, cookie);
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
pub fn enqueue_used(ring: &mut RingHandle, addr: usize, len: u32, cookie: *mut c_void) -> Result<(), &'static str> {
    return enqueue(ring.used_ring, addr, len, cookie);
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
pub fn dequeue_free(ring: &mut RingHandle, addr: &mut usize, len: &mut u32, cookie: *mut *mut c_void) -> Result<(), &'static str> {
    return dequeue(ring.free_ring, addr, len, cookie);
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
pub fn dequeue_used(ring: &mut RingHandle, addr: &mut usize, len: &mut u32, cookie: *mut *mut c_void) -> Result<(), &'static str> {
    return dequeue(ring.used_ring, addr, len, cookie);
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
pub fn ring_init(ring: &mut RingHandle, free: *mut RingBuffer, used: *mut RingBuffer, notify: Notify, buffer_init: bool) {
    ring.free_ring = free;
    ring.used_ring = used;
    ring.notify = notify;

    if buffer_init {
        unsafe {
            (*ring.free_ring).write_idx = 0;
            (*ring.free_ring).read_idx = 0;
            (*ring.used_ring).write_idx = 0;
            (*ring.used_ring).read_idx = 0;
        }
    }
}
