// kUI port runner + quit watcher (clean-room).
// Launches a port in its OWN session — so the whole tree (bash + game +
// any gptokeyb) is one killable process group — and watches the joystick
// node /dev/input/js0 for the MENU(guide)+START chord. On the chord it
// SIGKILLs the port's group; it also returns as soon as the port exits on
// its own. This gives every port a reliable MENU+START quit even when the
// port never runs gptokeyb (native-input games) or its gptokeyb kill
// misfires.
//
// Why js0 and setsid(2): the legacy joydev node is world-readable and,
// unlike the evdev event node, immune to EVIOCGRAB — so we still see the
// chord when a port's gptokeyb grabs the evdev device. The device busybox
// has no `setsid`, so the new session is made here with the syscall.
// Button numbers are the "TRIMUI Player1" (045e:028e) SDL joystick order
// from gamecontrollerdb: start = b7, guide = b8.
//
// Usage: kui_portrun <program> [args...]   (env is inherited)
// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <unistd.h>
#include <sys/wait.h>

struct js_event {
    uint32_t time;
    int16_t value;
    uint8_t type;
    uint8_t number;
};

#define JS_EVENT_BUTTON 0x01 // set (with 0x80) on the initial-state sync too
#define BTN_START 7
#define BTN_GUIDE 8

int main(int argc, char **argv) {
    if (argc < 2)
        return 2;

    pid_t child = fork();
    if (child < 0)
        return 3;
    if (child == 0) {
        // Own session => own process group; the port and its children
        // share it, so killing the group takes the whole tree.
        setsid();
        execvp(argv[1], &argv[1]);
        _exit(127);
    }

    // Parent watches the chord. It is NOT in the child's new group, so
    // killing that group never touches this process or session.sh above it.
    int fd = open("/dev/input/js0", O_RDONLY | O_NONBLOCK);
    struct pollfd pfd = {.fd = fd, .events = POLLIN};
    int start = 0, guide = 0, st;
    struct js_event e;

    for (;;) {
        if (waitpid(child, &st, WNOHANG) == child)
            break; // port exited on its own (incl. via its own gptokeyb)
        if (fd < 0) {
            waitpid(child, &st, 0); // no js0: just wait it out
            break;
        }
        if (poll(&pfd, 1, 200) <= 0)
            continue;
        while (read(fd, &e, sizeof(e)) == (ssize_t)sizeof(e)) {
            if (!(e.type & JS_EVENT_BUTTON))
                continue;
            if (e.number == BTN_START)
                start = e.value ? 1 : 0;
            else if (e.number == BTN_GUIDE)
                guide = e.value ? 1 : 0;
        }
        if (start && guide) {
            kill(-child, SIGKILL);
            waitpid(child, &st, 0);
            break;
        }
    }

    // Whether we killed on the chord or the port's own gptokeyb did, the
    // player is likely still holding MENU+START. Wait for release before
    // returning, else session.sh relaunches the launcher on a held MENU
    // and it opens the quick menu instead of landing on the carousel.
    // Bounded so a missed release can never hang the handoff.
    if (fd >= 0) {
        for (int waited = 0; (start || guide) && waited < 2000; waited += 100) {
            if (poll(&pfd, 1, 100) <= 0)
                continue;
            while (read(fd, &e, sizeof(e)) == (ssize_t)sizeof(e)) {
                if (!(e.type & JS_EVENT_BUTTON))
                    continue;
                if (e.number == BTN_START)
                    start = e.value ? 1 : 0;
                else if (e.number == BTN_GUIDE)
                    guide = e.value ? 1 : 0;
            }
        }
        close(fd);
    }
    return 0;
}
