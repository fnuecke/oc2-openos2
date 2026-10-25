/* SPDX-License-Identifier: MIT */
/* 16550 UART registers, for console.c and serial.c. */
#ifndef UART_H
#define UART_H

#define UART_RBR 0
#define UART_THR 0
#define UART_DLL 0
#define UART_IER 1
#define UART_DLM 1
#define UART_FCR 2
#define UART_LCR 3
#define UART_LSR 5
#define LSR_DR 0x01
#define LSR_THRE 0x20
#define LCR_8N1 0x03
#define LCR_DLAB 0x80
#define IER_RX 0x01
#define IER_TX 0x02

#endif
