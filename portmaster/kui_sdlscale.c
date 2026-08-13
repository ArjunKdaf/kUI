// kUI ports scaling shim (LD_PRELOAD).
// The vendor SDL "mali" backend hands every fullscreen window the
// panel surface (1024x768) regardless of the requested size, so games
// render unscaled in the top-left corner. Capture the size each game
// *asks* for at SDL_CreateWindow; when a renderer is created on that
// window and the request was smaller than the panel, apply SDL's
// logical scaling so the game's output fills the screen.
// SPDX-License-Identifier: GPL-3.0-or-later
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>

typedef void SDL_Window;
typedef void SDL_Renderer;

#define PANEL_W 1024
#define PANEL_H 768
#define MAXWIN 8

static SDL_Window *wins[MAXWIN];
static int req_w[MAXWIN], req_h[MAXWIN];

static SDL_Window *(*real_createwin)(const char *, int, int, int, int,
                                     unsigned int) = 0;
static SDL_Renderer *(*real_createren)(SDL_Window *, int, unsigned int) = 0;
static int (*set_logical)(SDL_Renderer *, int, int) = 0;

SDL_Window *SDL_CreateWindow(const char *title, int x, int y, int w, int h,
                             unsigned int flags) {
    if (!real_createwin)
        real_createwin = dlsym(RTLD_NEXT, "SDL_CreateWindow");
    SDL_Window *win = real_createwin ? real_createwin(title, x, y, w, h, flags) : 0;
    if (win) {
        for (int i = 0; i < MAXWIN; i++) {
            if (!wins[i] || wins[i] == win) {
                wins[i] = win;
                req_w[i] = w;
                req_h[i] = h;
                break;
            }
        }
    }
    return win;
}

SDL_Renderer *SDL_CreateRenderer(SDL_Window *win, int index, unsigned int flags) {
    if (!real_createren) {
        real_createren = dlsym(RTLD_NEXT, "SDL_CreateRenderer");
        set_logical = dlsym(RTLD_NEXT, "SDL_RenderSetLogicalSize");
    }
    SDL_Renderer *r = real_createren ? real_createren(win, index, flags) : 0;
    if (r && set_logical) {
        for (int i = 0; i < MAXWIN; i++) {
            if (wins[i] == win && req_w[i] > 0 && req_h[i] > 0
                && (req_w[i] < PANEL_W || req_h[i] < PANEL_H)) {
                set_logical(r, req_w[i], req_h[i]);
                fprintf(stderr, "[kui-scale] logical %dx%d -> %dx%d panel\n",
                        req_w[i], req_h[i], PANEL_W, PANEL_H);
                break;
            }
        }
    }
    return r;
}
