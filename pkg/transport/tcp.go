package transport

import (
	"bufio"
	"context"
	"net"
	"time"
)

type Listener struct {
	addr         string
	out          chan<- []byte
	reconnect    time.Duration
	reconnectMax time.Duration
	bufSize      int
	dialTimeout  time.Duration
	readTimeout  time.Duration
	errorHandler func(error)
}

type Option func(*Listener)

func WithReconnectInterval(d time.Duration) Option {
	return func(l *Listener) {
		if d > 0 {
			l.reconnect = d
		}
	}
}

func WithReconnectMax(d time.Duration) Option {
	return func(l *Listener) {
		if d > 0 {
			l.reconnectMax = d
		}
	}
}

func WithBufferSize(n int) Option {
	return func(l *Listener) {
		if n > 0 {
			l.bufSize = n
		}
	}
}

func WithDialTimeout(d time.Duration) Option {
	return func(l *Listener) {
		if d > 0 {
			l.dialTimeout = d
		}
	}
}

func WithReadTimeout(d time.Duration) Option {
	return func(l *Listener) {
		if d > 0 {
			l.readTimeout = d
		}
	}
}

func WithErrorHandler(fn func(error)) Option {
	return func(l *Listener) {
		if fn != nil {
			l.errorHandler = fn
		}
	}
}

func StartListener(ctx context.Context, addr string, out chan<- []byte, opts ...Option) *Listener {
	l := &Listener{
		addr:         addr,
		out:          out,
		reconnect:    1 * time.Second,
		reconnectMax: 30 * time.Second,
		bufSize:      64 * 1024,
		dialTimeout:  5 * time.Second,
	}
	for _, opt := range opts {
		opt(l)
	}
	go l.run(ctx)
	return l
}

func (l *Listener) run(ctx context.Context) {
	attempt := 0
	for {
		if ctx.Err() != nil {
			return
		}

		conn, err := net.DialTimeout("tcp", l.addr, l.dialTimeout)
		if err != nil {
			l.handleError(err)
			attempt++
			l.sleepBackoff(ctx, attempt)
			continue
		}

		attempt = 0
		err = l.handleConn(ctx, conn)
		_ = conn.Close()
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			l.handleError(err)
		}
		l.sleepBackoff(ctx, 1)
	}
}

func (l *Listener) handleConn(ctx context.Context, conn net.Conn) error {
	reader := bufio.NewReaderSize(conn, l.bufSize)
	for {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		if l.readTimeout > 0 {
			_ = conn.SetReadDeadline(time.Now().Add(l.readTimeout))
		}
		frame, err := reader.ReadBytes(0x00)
		if err != nil {
			if nerr, ok := err.(net.Error); ok && nerr.Timeout() {
				continue
			}
			return err
		}

		if len(frame) == 0 {
			continue
		}
		if frame[len(frame)-1] == 0x00 {
			frame = frame[:len(frame)-1]
		}
		if len(frame) == 0 {
			continue
		}
		payload := append([]byte(nil), frame...)
		select {
		case l.out <- payload:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

func (l *Listener) sleepBackoff(ctx context.Context, attempt int) {
	wait := min(l.reconnect*time.Duration(attempt), l.reconnectMax)
	timer := time.NewTimer(wait)
	select {
	case <-ctx.Done():
	case <-timer.C:
	}
	timer.Stop()
}

func (l *Listener) handleError(err error) {
	if l.errorHandler != nil {
		l.errorHandler(err)
	}
}
