/* SPDX-License-Identifier: MIT */
/*
 * malloc and friends over Matthew Conte's TLSF (external/tlsf): O(1) allocation and free with
 * immediate coalescing. The first heap region holds TLSF's control structure, every later region,
 * e.g. another memory stick, is a pool of its own.
 */

#include <stdlib.h>
#include <string.h>
#include "internal.h"
#include "tlsf.h"

static tlsf_t tlsf;
static size_t total; /* bytes in pools, without TLSF's overheads */
static size_t used; /* allocated blocks with their headers */

static size_t size_on_heap(void *ptr) {
    return ptr != NULL ? tlsf_block_size(ptr) + tlsf_alloc_overhead() : 0;
}

void heap_add(void *start, size_t size) {
    const size_t limit = tlsf_block_size_max() - tlsf_pool_overhead();
    u64 at = ((u64) start + 7) & ~7ull; /* TLSF wants 8-byte aligned pools of whole 8-byte words */
    size = (size - (size_t) (at - (u64) start)) & ~(size_t) 7;
    if (tlsf == NULL) {
        if (size <= tlsf_size() + tlsf_pool_overhead() + tlsf_block_size_min()) {
            return;
        }
        tlsf = tlsf_create((void *) at);
        at += tlsf_size();
        size -= tlsf_size();
    }
    while (size >= tlsf_pool_overhead() + tlsf_block_size_min()) {
        const size_t chunk = size < limit ? size : limit;
        if (tlsf_add_pool(tlsf, (void *) at, chunk) == NULL) {
            return;
        }
        total += chunk - tlsf_pool_overhead();
        at += chunk;
        size -= chunk;
    }
}

size_t heap_total(void) {
    return total;
}

size_t heap_used(void) {
    return used;
}

void *malloc(const size_t size) {
    void *ptr = size != 0 ? tlsf_malloc(tlsf, size) : NULL;
    used += size_on_heap(ptr);
    return ptr;
}

void free(void *ptr) {
    used -= size_on_heap(ptr);
    tlsf_free(tlsf, ptr);
}

void *realloc(void *ptr, const size_t size) {
    if (ptr != NULL && size == 0) {
        free(ptr);
        return NULL;
    }
    const size_t before = size_on_heap(ptr);
    void *moved = tlsf_realloc(tlsf, ptr, size);
    if (moved != NULL) {
        used += size_on_heap(moved) - before;
    }
    return moved;
}

void *calloc(const size_t count, const size_t size) {
    if (size != 0 && count > (size_t) -1 / size) {
        return NULL;
    }
    void *ptr = malloc(count * size);
    if (ptr != NULL) {
        memset(ptr, 0, count * size);
    }
    return ptr;
}
