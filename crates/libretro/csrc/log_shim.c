/* Variadic bridge for the libretro log interface.
 *
 * Cores call retro_log_printf_t as a C variadic function:
 *
 *     log_cb(RETRO_LOG_ERROR, "Unsupported game %s\n", path);
 *
 * Stable Rust can DECLARE a variadic `extern "C"` fn but cannot DEFINE one,
 * so a Rust callback can only ever see the fixed arguments -- it gets the
 * raw format string and the arguments are lost. That turned every core
 * message into a useless "core[0] %s", which hid a core's own diagnostics
 * exactly when they were needed most.
 *
 * This shim is the variadic function the core is handed. It expands the
 * arguments with vsnprintf and passes one finished string back to Rust.
 */
#include <stdarg.h>
#include <stdio.h>

/* implemented on the Rust side */
void kui_core_log_line(unsigned level, const char *msg);

void kui_core_log_shim(unsigned level, const char *fmt, ...) {
	char buf[2048];
	va_list ap;

	if (!fmt) {
		return;
	}
	va_start(ap, fmt);
	/* vsnprintf always NUL-terminates within the buffer and returns the
	 * length it WOULD have written; over-long lines are simply clipped. */
	if (vsnprintf(buf, sizeof buf, fmt, ap) < 0) {
		buf[0] = '\0';
	}
	va_end(ap);

	kui_core_log_line(level, buf);
}
