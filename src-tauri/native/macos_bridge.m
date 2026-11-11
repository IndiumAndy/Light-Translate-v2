#import "macos_bridge.h"
#import <ApplicationServices/ApplicationServices.h>
#import <Carbon/Carbon.h>
#import <Cocoa/Cocoa.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <Vision/Vision.h>
#import <objc/runtime.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// ============================================================================
// Non-Activating NSPanel Subclass for Zero-Focus-Steal Overlay
// ============================================================================

@interface LTNonActivatingPanel : NSPanel
@end

@implementation LTNonActivatingPanel
- (BOOL)canBecomeKeyWindow {
    return NO;
}
- (BOOL)canBecomeMainWindow {
    return NO;
}
- (BOOL)_preventsActivation {
    return YES;
}
@end

// ============================================================================
// Helpers: String Allocation & Screen Coordinate Mapping
// ============================================================================

static char *lt_strdup_ns(NSString *str) {
    if (!str) return NULL;
    const char *utf8 = [str UTF8String];
    if (!utf8) return NULL;
    size_t len = strlen(utf8);
    char *copy = (char *)malloc(len + 1);
    if (copy) {
        memcpy(copy, utf8, len + 1);
    }
    return copy;
}

static char *lt_json_encode(NSDictionary *dict) {
    if (!dict) return NULL;
    NSError *err = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:dict options:0 error:&err];
    if (!data || err) return NULL;
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    return lt_strdup_ns(json);
}

void lt_macos_free_string(char *ptr) {
    if (ptr) {
        free(ptr);
    }
}

static CGFloat lt_primary_screen_height(void) {
    CGRect bounds = CGDisplayBounds(CGMainDisplayID());
    return bounds.size.height > 0 ? bounds.size.height : 900.0;
}

// Convert AppKit bottom-left point -> unified top-left CG/AX point
static CGPoint lt_appkit_to_topleft(NSPoint pt) {
    CGFloat primaryH = lt_primary_screen_height();
    return CGPointMake(pt.x, primaryH - pt.y);
}

// Convert top-left rect -> AppKit bottom-left rect (for NSWindow setFrame:)
static NSRect lt_topleft_rect_to_appkit(double x, double y, double w, double h) {
    CGFloat primaryH = lt_primary_screen_height();
    return NSMakeRect(x, primaryH - y - h, w, h);
}

// ============================================================================
// Global Input Monitor (Modifiers, Double-Tap, Pointer, Hotkeys)
// ============================================================================

static lt_input_event_cb g_input_cb = NULL;
static int32_t g_trigger_modifier = 0; // 0=AltOrOption, 1=LeftOption, 2=RightOption, 3=Control
static bool g_double_tap_enabled = true;

static bool g_trigger_down = false;
static bool g_sentence_mod_down = false;
static bool g_other_key_during_tap = false;
static CFAbsoluteTime g_last_tap_release_time = 0.0;
static CGPoint g_tap_press_point = {0, 0};
static CFAbsoluteTime g_tap_press_time = 0.0;
static uint32_t g_PopupWindowNumber = 0;

// Device-dependent modifier masks in CGEventFlags / NSEventModifierFlags
#ifndef NX_DEVICELALTKEYMASK
#define NX_DEVICELALTKEYMASK 0x00000020
#define NX_DEVICERALTKEYMASK 0x00000040
#define NX_DEVICELCTLKEYMASK 0x00000001
#define NX_DEVICERCTLKEYMASK 0x00002000
#endif

static void lt_emit_event(NSDictionary *payload) {
    if (!g_input_cb) return;
    char *json = lt_json_encode(payload);
    if (json) {
        g_input_cb(json);
        free(json);
    }
}

static CGPoint lt_current_topleft_pointer(void) {
    CGEventRef ev = CGEventCreate(NULL);
    if (ev) {
        CGPoint loc = CGEventGetLocation(ev);
        CFRelease(ev);
        return loc;
    }
    return lt_appkit_to_topleft([NSEvent mouseLocation]);
}

void lt_macos_get_pointer(double *out_x, double *out_y) {
    CGPoint pt = lt_current_topleft_pointer();
    if (out_x) *out_x = pt.x;
    if (out_y) *out_y = pt.y;
}

void lt_macos_get_monitor_bounds(double x, double y, double *out_x, double *out_y, double *out_w, double *out_h) {
    CGPoint pt = CGPointMake(x, y);
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) == kCGErrorSuccess && count > 0) {
        CGDirectDisplayID displays[32];
        uint32_t cap = count > 32 ? 32 : count;
        if (CGGetActiveDisplayList(cap, displays, &cap) == kCGErrorSuccess) {
            for (uint32_t i = 0; i < cap; i++) {
                CGRect b = CGDisplayBounds(displays[i]);
                if (CGRectContainsPoint(b, pt)) {
                    *out_x = b.origin.x;
                    *out_y = b.origin.y;
                    *out_w = b.size.width;
                    *out_h = b.size.height;
                    return;
                }
            }
        }
    }
    CGRect mainB = CGDisplayBounds(CGMainDisplayID());
    *out_x = mainB.origin.x;
    *out_y = mainB.origin.y;
    *out_w = mainB.size.width;
    *out_h = mainB.size.height;
}

static bool lt_is_trigger_active(NSEventModifierFlags flags) {
    switch (g_trigger_modifier) {
        case 1: // LeftOption
            return (flags & NSEventModifierFlagOption) && ((flags & NX_DEVICELALTKEYMASK) != 0);
        case 2: // RightOption
            return (flags & NSEventModifierFlagOption) && ((flags & NX_DEVICERALTKEYMASK) != 0);
        case 3: // Control
            return (flags & NSEventModifierFlagControl) != 0;
        case 0: // AltOrOption
        default:
            return (flags & NSEventModifierFlagOption) != 0;
    }
}

static bool lt_is_sentence_mod_active(NSEventModifierFlags flags) {
    if (g_trigger_modifier == 3) {
        // When Control is primary trigger, Option acts as sentence modifier
        return (flags & NSEventModifierFlagOption) != 0;
    }
    return (flags & NSEventModifierFlagControl) != 0;
}

static void lt_handle_ns_event(NSEvent *event) {
    if (!event) return;
    NSEventType type = [event type];
    CGPoint pt = lt_current_topleft_pointer();

    if (type == NSEventTypeFlagsChanged) {
        NSEventModifierFlags flags = [event modifierFlags];
        // Ignore Command-chord shortcuts so Cmd+Tab / Cmd+Option+I never false-trigger hover!
        bool cmdHeld = (flags & NSEventModifierFlagCommand) != 0;
        bool nowTrigger = !cmdHeld && lt_is_trigger_active(flags);
        bool nowSentence = lt_is_sentence_mod_active(flags);

        if (nowTrigger && !g_trigger_down) {
            g_trigger_down = true;
            g_sentence_mod_down = nowSentence;
            g_other_key_during_tap = nowSentence;
            g_tap_press_time = CFAbsoluteTimeGetCurrent();
            g_tap_press_point = pt;

            lt_emit_event(@{
                @"kind": @"triggerPressed",
                @"sentenceModifier": @(nowSentence),
                @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
            });
        } else if (!nowTrigger && g_trigger_down) {
            g_trigger_down = false;
            g_sentence_mod_down = false;
            CFAbsoluteTime now = CFAbsoluteTimeGetCurrent();

            lt_emit_event(@{
                @"kind": @"triggerReleased",
                @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
            });

            // Check clean double-tap for selection translation
            double tapDuration = now - g_tap_press_time;
            double moveDist = hypot(pt.x - g_tap_press_point.x, pt.y - g_tap_press_point.y);
            if (g_double_tap_enabled && !g_other_key_during_tap && !cmdHeld && tapDuration < 0.28 && moveDist < 12.0) {
                if (now - g_last_tap_release_time <= 0.36) {
                    g_last_tap_release_time = 0.0;
                    lt_emit_event(@{
                        @"kind": @"doubleTapSelection",
                        @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
                    });
                } else {
                    g_last_tap_release_time = now;
                }
            } else {
                g_last_tap_release_time = 0.0;
            }
        } else if (g_trigger_down && nowSentence != g_sentence_mod_down) {
            g_sentence_mod_down = nowSentence;
            g_other_key_during_tap = true;
            lt_emit_event(@{
                @"kind": @"sentenceModifierChanged",
                @"active": @(nowSentence),
                @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
            });
        }
        return;
    }

    if (type == NSEventTypeKeyDown) {
        g_other_key_during_tap = true;
        unsigned short keyCode = [event keyCode];
        NSEventModifierFlags flags = [event modifierFlags];

        if (keyCode == kVK_Escape) {
            lt_emit_event(@{ @"kind": @"escapePressed" });
            return;
        }

        bool opt = (flags & NSEventModifierFlagOption) != 0;
        bool cmd = (flags & NSEventModifierFlagCommand) != 0;
        bool ctrl = (flags & NSEventModifierFlagControl) != 0;

        // Alt+Cmd+T -> Selection translate shortcut
        if (opt && cmd && !ctrl && keyCode == kVK_ANSI_T) {
            lt_emit_event(@{
                @"kind": @"shortcutSelection",
                @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
            });
            return;
        }

        // Ctrl+Cmd+S -> Region OCR capture shortcut
        if (ctrl && cmd && !opt && keyCode == kVK_ANSI_S) {
            lt_emit_event(@{ @"kind": @"shortcutCapture" });
            return;
        }
        return;
    }

    if (type == NSEventTypeMouseMoved || type == NSEventTypeLeftMouseDragged || type == NSEventTypeRightMouseDragged) {
        lt_emit_event(@{
            @"kind": @"pointerMoved",
            @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
        });
        return;
    }

    if (type == NSEventTypeLeftMouseDown || type == NSEventTypeRightMouseDown || type == NSEventTypeOtherMouseDown) {
        g_other_key_during_tap = true;
        lt_emit_event(@{
            @"kind": @"globalMouseDown",
            @"pointer": @{ @"x": @(pt.x), @"y": @(pt.y) }
        });
    }
}

void lt_macos_start_input_monitor(lt_input_event_cb callback) {
    g_input_cb = callback;
    dispatch_async(dispatch_get_main_queue(), ^{
        NSEventMask mask = NSEventMaskFlagsChanged
                         | NSEventMaskKeyDown
                         | NSEventMaskMouseMoved
                         | NSEventMaskLeftMouseDragged
                         | NSEventMaskLeftMouseDown
                         | NSEventMaskRightMouseDown
                         | NSEventMaskOtherMouseDown;

        [NSEvent addGlobalMonitorForEventsMatchingMask:mask handler:^(NSEvent *event) {
            lt_handle_ns_event(event);
        }];

        [NSEvent addLocalMonitorForEventsMatchingMask:mask handler:^NSEvent *(NSEvent *event) {
            lt_handle_ns_event(event);
            return event;
        }];
    });
}

void lt_macos_update_trigger_config(int32_t modifier_kind, bool double_tap_selection) {
    g_trigger_modifier = modifier_kind;
    g_double_tap_enabled = double_tap_selection;
}

// ============================================================================
// Permissions & Clipboard
// ============================================================================

bool lt_macos_check_accessibility(bool prompt) {
    if (prompt) {
        NSDictionary *opts = @{ (__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES };
        return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)opts);
    }
    return AXIsProcessTrusted();
}

bool lt_macos_check_screen_recording(void) {
    return CGPreflightScreenCaptureAccess();
}

bool lt_macos_write_clipboard(const char *text_utf8) {
    if (!text_utf8) return false;
    NSString *str = [NSString stringWithUTF8String:text_utf8];
    if (!str) return false;
    NSPasteboard *pb = [NSPasteboard generalPasteboard];
    [pb clearContents];
    return [pb setString:str forType:NSPasteboardTypeString];
}

// ============================================================================
// Window Discovery (Relaxed Layer Rules: Supports Popovers/Menus/Floating Panels)
// ============================================================================

typedef struct {
    pid_t pid;
    uint32_t window_id;
    CGRect bounds;
    bool valid;
} LTWindowHit;

static LTWindowHit lt_topmost_window_at(CGPoint pt) {
    LTWindowHit res = {0, 0, CGRectZero, false};
    pid_t selfPid = getpid();
    CFArrayRef list = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID
    );
    if (!list) return res;

    CFIndex count = CFArrayGetCount(list);
    for (CFIndex i = 0; i < count; i++) {
        NSDictionary *info = (__bridge NSDictionary *)CFArrayGetValueAtIndex(list, i);
        if (![info isKindOfClass:[NSDictionary class]]) continue;
        pid_t ownerPid = [info[(id)kCGWindowOwnerPID] intValue];
        uint32_t winNum = [info[(id)kCGWindowNumber] unsignedIntValue];
        int layer = [info[(id)kCGWindowLayer] intValue];
        double alpha = info[(id)kCGWindowAlpha] ? [info[(id)kCGWindowAlpha] doubleValue] : 1.0;

        if (ownerPid <= 0 || ownerPid == selfPid || alpha <= 0.01) continue;
        // Allow normal windows (0) plus floating panels, popovers, and context menus (<= 110);
        // exclude system lockscreen/screensaver/security shield layers (>= 1000).
        if (layer < 0 || layer > 110) continue;

        NSDictionary *boundsDict = info[(id)kCGWindowBounds];
        CGRect rect = CGRectZero;
        if (!boundsDict || !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)boundsDict, &rect)) {
            continue;
        }
        if (rect.size.width < 4 || rect.size.height < 4) continue;

        if (CGRectContainsPoint(rect, pt)) {
            NSString *ownerName = info[(id)kCGWindowOwnerName] ?: @"";
            if ([ownerName isEqualToString:@"Window Server"] ||
                [ownerName isEqualToString:@"loginwindow"] ||
                [ownerName isEqualToString:@"ScreenSaverEngine"]) {
                continue;
            }
            res.pid = ownerPid;
            res.window_id = winNum;
            res.bounds = rect;
            res.valid = true;
            break;
        }
    }
    CFRelease(list);
    return res;
}

static NSString *lt_bundle_id_for_pid(pid_t pid) {
    if (pid <= 0) return nil;
    NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
    return app.bundleIdentifier ?: app.localizedName;
}

char *lt_macos_app_id_at_point(double x, double y) {
    LTWindowHit hit = lt_topmost_window_at(CGPointMake(x, y));
    if (!hit.valid) return NULL;
    return lt_strdup_ns(lt_bundle_id_for_pid(hit.pid));
}

char *lt_macos_focused_app_id(void) {
    NSRunningApplication *front = [[NSWorkspace sharedWorkspace] frontmostApplication];
    if (!front || front.processIdentifier == getpid()) return NULL;
    return lt_strdup_ns(front.bundleIdentifier ?: front.localizedName);
}

// ============================================================================
// Accessibility Helpers + Auto-Wake Chromium/Electron + Subtree & Paragraph Traversal
// ============================================================================

static NSMutableSet<NSNumber *> *lt_awakened_pids(void) {
    static NSMutableSet<NSNumber *> *set = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        set = [NSMutableSet set];
    });
    return set;
}

static void lt_ensure_app_ax_awake(pid_t pid, NSString *bundleId) {
    if (pid <= 0) return;
    NSMutableSet<NSNumber *> *awakened = lt_awakened_pids();
    @synchronized (awakened) {
        if ([awakened containsObject:@(pid)]) return;
        [awakened addObject:@(pid)];
    }

    AXUIElementRef appElem = AXUIElementCreateApplication(pid);
    if (!appElem) return;
    AXUIElementSetMessagingTimeout(appElem, 0.25f);

    // Inject AXManualAccessibility = true to wake Chromium, Electron, Firefox, Arc, Brave, VS Code, Notion, etc.
    AXUIElementSetAttributeValue(appElem, CFSTR("AXManualAccessibility"), kCFBooleanTrue);

    // For known Chromium/Electron bundles, also set AXEnhancedUserInterface if needed
    NSString *lower = (bundleId ?: @"").lowercaseString;
    if ([lower containsString:@"chrome"] ||
        [lower containsString:@"chromium"] ||
        [lower containsString:@"brave"] ||
        [lower containsString:@"edgemac"] ||
        [lower containsString:@"arc"] ||
        [lower containsString:@"vivaldi"] ||
        [lower containsString:@"vscode"] ||
        [lower containsString:@"electron"] ||
        [lower containsString:@"notion"] ||
        [lower containsString:@"obsidian"] ||
        [lower containsString:@"cursor"]) {
        AXUIElementSetAttributeValue(appElem, CFSTR("AXEnhancedUserInterface"), kCFBooleanTrue);
    }
    CFRelease(appElem);
}

static NSString *lt_ax_string(AXUIElementRef elem, CFStringRef attr) {
    if (!elem) return nil;
    CFTypeRef val = NULL;
    if (AXUIElementCopyAttributeValue(elem, attr, &val) != kAXErrorSuccess || !val) {
        return nil;
    }
    NSString *res = nil;
    if (CFGetTypeID(val) == CFStringGetTypeID()) {
        res = [(__bridge NSString *)val copy];
    } else if (CFGetTypeID(val) == CFAttributedStringGetTypeID()) {
        res = [[(__bridge NSAttributedString *)val string] copy];
    }
    CFRelease(val);
    return res;
}

static bool lt_ax_is_protected(AXUIElementRef elem) {
    NSString *role = lt_ax_string(elem, kAXRoleAttribute);
    NSString *subrole = lt_ax_string(elem, kAXSubroleAttribute);
    if ([subrole isEqualToString:@"AXSecureTextField"]) return true;
    if (role && [role hasSuffix:@"SecureTextField"]) return true;
    return false;
}

static bool lt_ax_frame(AXUIElementRef elem, CGRect *outRect) {
    if (!elem) return false;
    CFTypeRef posVal = NULL;
    CFTypeRef sizeVal = NULL;
    bool ok = false;
    if (AXUIElementCopyAttributeValue(elem, kAXPositionAttribute, &posVal) == kAXErrorSuccess &&
        AXUIElementCopyAttributeValue(elem, kAXSizeAttribute, &sizeVal) == kAXErrorSuccess &&
        posVal && sizeVal &&
        CFGetTypeID(posVal) == AXValueGetTypeID() &&
        CFGetTypeID(sizeVal) == AXValueGetTypeID()) {
        CGPoint origin = CGPointZero;
        CGSize size = CGSizeZero;
        if (AXValueGetValue((AXValueRef)posVal, kAXValueCGPointType, &origin) &&
            AXValueGetValue((AXValueRef)sizeVal, kAXValueCGSizeType, &size) &&
            size.width > 0 && size.height > 0 && isfinite(origin.x) && isfinite(origin.y)) {
            *outRect = CGRectMake(origin.x, origin.y, size.width, size.height);
            ok = true;
        }
    }
    if (posVal) CFRelease(posVal);
    if (sizeVal) CFRelease(sizeVal);
    return ok;
}

static NSString *lt_ax_string_for_range(AXUIElementRef elem, CFRange range) {
    AXValueRef param = AXValueCreate(kAXValueCFRangeType, &range);
    if (!param) return nil;
    CFTypeRef val = NULL;
    NSString *res = nil;
    if (AXUIElementCopyParameterizedAttributeValue(elem, kAXStringForRangeParameterizedAttribute, param, &val) == kAXErrorSuccess && val) {
        if (CFGetTypeID(val) == CFStringGetTypeID()) {
            res = [(__bridge NSString *)val copy];
        } else if (CFGetTypeID(val) == CFAttributedStringGetTypeID()) {
            res = [[(__bridge NSAttributedString *)val string] copy];
        }
        CFRelease(val);
    }
    CFRelease(param);
    return res;
}

static bool lt_ax_bounds_for_range(AXUIElementRef elem, CFRange range, CGRect *outRect) {
    AXValueRef param = AXValueCreate(kAXValueCFRangeType, &range);
    if (!param) return false;
    CFTypeRef val = NULL;
    bool ok = false;
    if (AXUIElementCopyParameterizedAttributeValue(elem, kAXBoundsForRangeParameterizedAttribute, param, &val) == kAXErrorSuccess && val) {
        if (CFGetTypeID(val) == AXValueGetTypeID()) {
            CGRect r = CGRectZero;
            if (AXValueGetValue((AXValueRef)val, kAXValueCGRectType, &r) && r.size.width > 0 && r.size.height > 0) {
                *outRect = r;
                ok = true;
            }
        }
        CFRelease(val);
    }
    CFRelease(param);
    return ok;
}

static bool lt_ax_range_for_position(AXUIElementRef elem, CGPoint pt, CFRange *outRange) {
    AXValueRef param = AXValueCreate(kAXValueCGPointType, &pt);
    if (!param) return false;
    CFTypeRef val = NULL;
    bool ok = false;
    if (AXUIElementCopyParameterizedAttributeValue(elem, kAXRangeForPositionParameterizedAttribute, param, &val) == kAXErrorSuccess && val) {
        if (CFGetTypeID(val) == AXValueGetTypeID()) {
            CFRange r = CFRangeMake(0, 0);
            if (AXValueGetValue((AXValueRef)val, kAXValueCFRangeType, &r) && r.location >= 0 && r.length >= 0) {
                *outRange = r;
                ok = true;
            }
        }
        CFRelease(val);
    }
    CFRelease(param);
    return ok;
}

static CFIndex lt_ax_char_count(AXUIElementRef elem) {
    CFTypeRef val = NULL;
    CFIndex count = -1;
    if (AXUIElementCopyAttributeValue(elem, kAXNumberOfCharactersAttribute, &val) == kAXErrorSuccess && val) {
        if (CFGetTypeID(val) == CFNumberGetTypeID()) {
            CFNumberGetValue((CFNumberRef)val, kCFNumberCFIndexType, &count);
        }
        CFRelease(val);
    }
    return count;
}

// ============================================================================
// Selection Extraction (AX Focused Element + Optional Safe Clipboard Fallback)
// ============================================================================

static NSString *lt_clipboard_fallback_copy(void) {
    NSPasteboard *pb = [NSPasteboard generalPasteboard];
    NSInteger initialChange = [pb changeCount];

    // 1. Snapshot all existing pasteboard items & types
    NSMutableArray<NSDictionary<NSPasteboardType, NSData *> *> *backup = [NSMutableArray array];
    for (NSPasteboardItem *item in ([pb pasteboardItems] ?: @[])) {
        NSMutableDictionary<NSPasteboardType, NSData *> *entry = [NSMutableDictionary dictionary];
        for (NSPasteboardType t in [item types]) {
            NSData *d = [item dataForType:t];
            if (d) entry[t] = [d copy];
        }
        if (entry.count > 0) [backup addObject:entry];
    }

    // 2. Synthesize Cmd+C
    CGEventSourceRef src = CGEventSourceCreate(kCGEventSourceStateCombinedSessionState);
    CGEventRef keyDown = CGEventCreateKeyboardEvent(src, (CGKeyCode)kVK_ANSI_C, true);
    CGEventRef keyUp = CGEventCreateKeyboardEvent(src, (CGKeyCode)kVK_ANSI_C, false);
    CGEventSetFlags(keyDown, kCGEventFlagMaskCommand);
    CGEventSetFlags(keyUp, kCGEventFlagMaskCommand);
    CGEventPost(kCGHIDEventTap, keyDown);
    CGEventPost(kCGHIDEventTap, keyUp);
    if (keyDown) CFRelease(keyDown);
    if (keyUp) CFRelease(keyUp);
    if (src) CFRelease(src);

    // 3. Poll up to ~35ms for pasteboard update
    NSString *copied = nil;
    for (int i = 0; i < 7; i++) {
        usleep(5000); // 5ms
        if ([pb changeCount] != initialChange) {
            copied = [[pb stringForType:NSPasteboardTypeString] copy];
            break;
        }
    }

    // 4. Restore original clipboard immediately
    [pb clearContents];
    if (backup.count > 0) {
        NSMutableArray<NSPasteboardItem *> *restored = [NSMutableArray arrayWithCapacity:backup.count];
        for (NSDictionary<NSPasteboardType, NSData *> *entry in backup) {
            NSPasteboardItem *item = [[NSPasteboardItem alloc] init];
            for (NSPasteboardType t in entry) {
                [item setData:entry[t] forType:t];
            }
            [restored addObject:item];
        }
        [pb writeObjects:restored];
    }

    return copied;
}

char *lt_macos_extract_selection_json(double x, double y, bool allow_clipboard_fallback) {
    (void)x;
    (void)y;
    if (!AXIsProcessTrusted()) {
        return lt_json_encode(@{ @"status": @"permissionDenied" });
    }

    NSRunningApplication *front = [[NSWorkspace sharedWorkspace] frontmostApplication];
    NSString *appId = front ? (front.bundleIdentifier ?: front.localizedName) : nil;
    if (front && front.processIdentifier != getpid()) {
        lt_ensure_app_ax_awake(front.processIdentifier, appId);
    }

    AXUIElementRef sys = AXUIElementCreateSystemWide();
    AXUIElementSetMessagingTimeout(sys, 0.25f);
    CFTypeRef focusedRef = NULL;
    AXError err = AXUIElementCopyAttributeValue(sys, kAXFocusedUIElementAttribute, &focusedRef);
    CFRelease(sys);

    if (err == kAXErrorSuccess && focusedRef && CFGetTypeID(focusedRef) == AXUIElementGetTypeID()) {
        AXUIElementRef focused = (AXUIElementRef)focusedRef;
        AXUIElementSetMessagingTimeout(focused, 0.25f);

        pid_t ownerPid = 0;
        AXUIElementGetPid(focused, &ownerPid);
        if (ownerPid == getpid()) {
            CFRelease(focusedRef);
            return lt_json_encode(@{ @"status": @"none" });
        }

        if (lt_ax_is_protected(focused)) {
            CFRelease(focusedRef);
            return lt_json_encode(@{ @"status": @"blockedSensitive" });
        }

        NSString *selText = lt_ax_string(focused, kAXSelectedTextAttribute);
        CGRect selBounds = CGRectZero;
        bool hasBounds = false;

        CFTypeRef rangeRef = NULL;
        if (AXUIElementCopyAttributeValue(focused, kAXSelectedTextRangeAttribute, &rangeRef) == kAXErrorSuccess && rangeRef) {
            if (CFGetTypeID(rangeRef) == AXValueGetTypeID()) {
                CFRange r = CFRangeMake(0, 0);
                if (AXValueGetValue((AXValueRef)rangeRef, kAXValueCFRangeType, &r) && r.length > 0) {
                    if (!selText || selText.length == 0) {
                        selText = lt_ax_string_for_range(focused, r);
                    }
                    hasBounds = lt_ax_bounds_for_range(focused, r, &selBounds);
                }
            }
            CFRelease(rangeRef);
        }

        CFRelease(focusedRef);

        NSString *trimmed = [selText stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (trimmed && trimmed.length > 0) {
            NSMutableDictionary *resp = [@{
                @"status": @"ok",
                @"text": trimmed,
                @"source": @"accessibility",
                @"appId": appId ?: [NSNull null]
            } mutableCopy];
            if (hasBounds) {
                resp[@"rect"] = @{
                    @"x": @(selBounds.origin.x),
                    @"y": @(selBounds.origin.y),
                    @"width": @(selBounds.size.width),
                    @"height": @(selBounds.size.height)
                };
            }
            return lt_json_encode(resp);
        }
    } else if (focusedRef) {
        CFRelease(focusedRef);
    }

    // Optional safe clipboard backup-copy-restore fallback
    if (allow_clipboard_fallback && front && front.processIdentifier != getpid()) {
        NSString *copied = lt_clipboard_fallback_copy();
        NSString *trimmed = [copied stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (trimmed && trimmed.length > 0) {
            return lt_json_encode(@{
                @"status": @"ok",
                @"text": trimmed,
                @"source": @"clipboardFallback",
                @"appId": appId ?: [NSNull null]
            });
        }
    }

    return lt_json_encode(@{ @"status": @"none" });
}

// ============================================================================
// 4-Level, 24-Node Bounded Subtree Search + Cross-Node Paragraph Aggregation
// ============================================================================

static NSString *lt_best_element_text(AXUIElementRef elem, NSString *role) {
    NSString *val = lt_ax_string(elem, kAXValueAttribute);
    NSString *title = lt_ax_string(elem, kAXTitleAttribute);
    NSString *desc = lt_ax_string(elem, kAXDescriptionAttribute);
    if ([role isEqualToString:@"AXStaticText"] || [role isEqualToString:@"AXTextArea"] || [role isEqualToString:@"AXTextField"]) {
        if (val.length > 0) return val;
        if (title.length > 0) return title;
    } else {
        if (title.length > 0) return title;
        if (val.length > 0) return val;
    }
    if (desc.length > 0 && desc.length <= 160) return desc;
    return nil;
}

// Bounded DFS (max depth 4, max 24 visited nodes) matching child geometry against `pt`
static AXUIElementRef lt_find_deepest_text_child_retained(
    AXUIElementRef root,
    CGPoint pt,
    pid_t expectedPid,
    int depth,
    int *visitedBudget
) {
    if (!root || depth > 4 || *visitedBudget <= 0) return NULL;

    CFTypeRef childrenRef = NULL;
    if (AXUIElementCopyAttributeValue(root, kAXChildrenAttribute, &childrenRef) != kAXErrorSuccess || !childrenRef) {
        return NULL;
    }
    if (CFGetTypeID(childrenRef) != CFArrayGetTypeID()) {
        CFRelease(childrenRef);
        return NULL;
    }

    CFArrayRef children = (CFArrayRef)childrenRef;
    CFIndex count = CFArrayGetCount(children);
    AXUIElementRef bestMatch = NULL;
    CGFloat bestArea = CGFLOAT_MAX;

    for (CFIndex i = 0; i < count && *visitedBudget > 0; i++) {
        AXUIElementRef child = (AXUIElementRef)CFArrayGetValueAtIndex(children, i);
        if (!child) continue;
        (*visitedBudget)--;
        AXUIElementSetMessagingTimeout(child, 0.2f);

        pid_t childPid = 0;
        if (AXUIElementGetPid(child, &childPid) == kAXErrorSuccess && childPid != expectedPid) {
            continue;
        }
        if (lt_ax_is_protected(child)) {
            continue;
        }

        CGRect frame = CGRectZero;
        bool hasFrame = lt_ax_frame(child, &frame);
        if (hasFrame) {
            CGRect expanded = CGRectInset(frame, -3.0, -3.0);
            if (!CGRectContainsPoint(expanded, pt)) {
                continue;
            }
        }

        // Recurse deeper first to prefer exact inline leaf (`<span>`, `<a>`, `AXStaticText`)
        AXUIElementRef deeper = lt_find_deepest_text_child_retained(child, pt, expectedPid, depth + 1, visitedBudget);
        if (deeper) {
            if (bestMatch) CFRelease(bestMatch);
            bestMatch = deeper;
            break;
        }

        NSString *role = lt_ax_string(child, kAXRoleAttribute) ?: @"";
        NSString *txt = lt_best_element_text(child, role);
        NSString *trimmed = [txt stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (trimmed.length > 0 && hasFrame) {
            CGFloat area = frame.size.width * frame.size.height;
            if (area < bestArea) {
                if (bestMatch) CFRelease(bestMatch);
                bestMatch = (AXUIElementRef)CFRetain(child);
                bestArea = area;
            }
        }
    }

    CFRelease(childrenRef);
    return bestMatch;
}

// Collect inline text descendants of a paragraph container, tracking the UTF-16 start offset of `targetElem`
static void lt_collect_paragraph_inline_text(
    AXUIElementRef node,
    AXUIElementRef targetElem,
    NSMutableString *buffer,
    NSInteger *outTargetStart,
    int depth,
    int *budget
) {
    if (!node || depth > 4 || *budget <= 0 || buffer.length >= 2000) return;
    (*budget)--;

    bool isTarget = CFEqual(node, targetElem);
    NSString *role = lt_ax_string(node, kAXRoleAttribute) ?: @"";

    if ([role isEqualToString:@"AXStaticText"]) {
        NSString *val = lt_ax_string(node, kAXValueAttribute) ?: lt_ax_string(node, kAXTitleAttribute);
        if (val.length > 0) {
            if (buffer.length > 0) {
                unichar prev = [buffer characterAtIndex:buffer.length - 1];
                unichar first = [val characterAtIndex:0];
                bool prevSpace = [[NSCharacterSet whitespaceAndNewlineCharacterSet] characterIsMember:prev];
                bool nextPunct = [[NSCharacterSet characterSetWithCharactersInString:@".,!?;:)]}”’"] characterIsMember:first];
                if (!prevSpace && !nextPunct) {
                    [buffer appendString:@" "];
                }
            }
            if (isTarget && *outTargetStart < 0) {
                *outTargetStart = (NSInteger)buffer.length;
            }
            if (buffer.length + val.length <= 2000) {
                [buffer appendString:val];
            }
        }
        return;
    }

    if (isTarget && *outTargetStart < 0) {
        *outTargetStart = (NSInteger)buffer.length;
    }

    CFTypeRef childrenRef = NULL;
    if (AXUIElementCopyAttributeValue(node, kAXChildrenAttribute, &childrenRef) != kAXErrorSuccess || !childrenRef) {
        return;
    }
    if (CFGetTypeID(childrenRef) == CFArrayGetTypeID()) {
        CFArrayRef arr = (CFArrayRef)childrenRef;
        CFIndex count = CFArrayGetCount(arr);
        for (CFIndex i = 0; i < count && *budget > 0 && buffer.length < 2000; i++) {
            AXUIElementRef child = (AXUIElementRef)CFArrayGetValueAtIndex(arr, i);
            lt_collect_paragraph_inline_text(child, targetElem, buffer, outTargetStart, depth + 1, budget);
        }
    }
    CFRelease(childrenRef);
}

// Climb 1~2 parent levels to assemble a full paragraph across fragmented DOM inline nodes (`<a>`, `<code>`, `<span>`)
static NSDictionary *lt_try_aggregate_parent_paragraph(AXUIElementRef leaf, NSInteger leafHitOffset) {
    AXUIElementRef current = (AXUIElementRef)CFRetain(leaf);
    NSDictionary *bestResult = nil;

    for (int climb = 0; climb < 2; climb++) {
        CFTypeRef parentRef = NULL;
        AXError err = AXUIElementCopyAttributeValue(current, kAXParentAttribute, &parentRef);
        CFRelease(current);
        if (err != kAXErrorSuccess || !parentRef || CFGetTypeID(parentRef) != AXUIElementGetTypeID()) {
            if (parentRef) CFRelease(parentRef);
            break;
        }
        AXUIElementRef parent = (AXUIElementRef)parentRef;
        AXUIElementSetMessagingTimeout(parent, 0.2f);
        NSString *pRole = lt_ax_string(parent, kAXRoleAttribute) ?: @"";

        // Stop before climbing into whole-window / scroll-area / web-root containers
        if ([pRole isEqualToString:@"AXWindow"] ||
            [pRole isEqualToString:@"AXScrollArea"] ||
            [pRole isEqualToString:@"AXWebArea"] ||
            [pRole isEqualToString:@"AXApplication"]) {
            CFRelease(parent);
            break;
        }

        CGRect pFrame = CGRectZero;
        if (lt_ax_frame(parent, &pFrame) && pFrame.size.height <= 420.0) {
            NSMutableString *buf = [NSMutableString string];
            NSInteger targetStart = -1;
            int budget = 36;
            lt_collect_paragraph_inline_text(parent, leaf, buf, &targetStart, 0, &budget);
            if (targetStart >= 0 && buf.length > 0) {
                bestResult = @{
                    @"paragraphText": [buf copy],
                    @"paragraphHitOffset": @(targetStart + leafHitOffset)
                };
                // If we already see sentence punctuation, 1~2 levels is enough
                if ([buf containsString:@"."] || [buf containsString:@"!"] || [buf containsString:@"?"] || [buf containsString:@"。"]) {
                    CFRelease(parent);
                    break;
                }
            }
        }
        current = parent; // transfers retain to next iteration
    }
    return bestResult;
}

char *lt_macos_extract_hover_ax_json(double x, double y) {
    if (!AXIsProcessTrusted()) {
        return lt_json_encode(@{ @"status": @"permissionDenied" });
    }

    CGPoint pt = CGPointMake(x, y);
    LTWindowHit winHit = lt_topmost_window_at(pt);
    if (!winHit.valid) {
        return lt_json_encode(@{ @"status": @"positionUnresolved" });
    }

    NSString *appId = lt_bundle_id_for_pid(winHit.pid);
    lt_ensure_app_ax_awake(winHit.pid, appId);

    AXUIElementRef sys = AXUIElementCreateSystemWide();
    AXUIElementSetMessagingTimeout(sys, 0.25f);
    AXUIElementRef hitElem = NULL;
    AXError err = AXUIElementCopyElementAtPosition(sys, (float)pt.x, (float)pt.y, &hitElem);
    CFRelease(sys);

    // Retry once after 25ms if we just woke up a lazy Chromium/Electron renderer tree!
    if (err != kAXErrorSuccess || !hitElem) {
        usleep(25000);
        AXUIElementRef sys2 = AXUIElementCreateSystemWide();
        AXUIElementSetMessagingTimeout(sys2, 0.25f);
        err = AXUIElementCopyElementAtPosition(sys2, (float)pt.x, (float)pt.y, &hitElem);
        CFRelease(sys2);
    }

    if (err != kAXErrorSuccess || !hitElem) {
        return lt_json_encode(@{ @"status": @"positionUnresolved", @"appId": appId ?: [NSNull null] });
    }

    AXUIElementSetMessagingTimeout(hitElem, 0.25f);
    pid_t elemPid = 0;
    AXUIElementGetPid(hitElem, &elemPid);
    if (elemPid == getpid()) {
        CFRelease(hitElem);
        return lt_json_encode(@{ @"status": @"positionUnresolved" });
    }
    if (lt_ax_is_protected(hitElem)) {
        CFRelease(hitElem);
        return lt_json_encode(@{ @"status": @"blockedSensitive", @"appId": appId ?: [NSNull null] });
    }

    // 1. Try direct parameterized range mapping at cursor
    CFRange charRange = CFRangeMake(0, 0);
    bool hasRange = lt_ax_range_for_position(hitElem, pt, &charRange);

    // 2. If direct element didn't resolve a character range, walk up to 4 levels / 24 children by rect!
    if (!hasRange) {
        int budget = 24;
        AXUIElementRef deepChild = lt_find_deepest_text_child_retained(hitElem, pt, winHit.pid, 1, &budget);
        if (deepChild) {
            CFRelease(hitElem);
            hitElem = deepChild;
            hasRange = lt_ax_range_for_position(hitElem, pt, &charRange);
        }
    }

    NSString *role = lt_ax_string(hitElem, kAXRoleAttribute) ?: @"";

    if (hasRange && charRange.length <= 128) {
        CFIndex totalLen = lt_ax_char_count(hitElem);
        CFIndex winStart = charRange.location > 900 ? (charRange.location - 900) : 0;
        CFIndex winLen = 1800;
        if (totalLen > 0 && winStart + winLen > totalLen) {
            winLen = totalLen - winStart;
        }

        NSString *windowText = nil;
        if (winLen > 0) {
            windowText = lt_ax_string_for_range(hitElem, CFRangeMake(winStart, winLen));
        }
        if (!windowText || windowText.length == 0) {
            NSString *fullVal = lt_best_element_text(hitElem, role);
            if (fullVal.length > 0 && charRange.location < (CFIndex)fullVal.length) {
                winStart = 0;
                totalLen = (CFIndex)fullVal.length;
                windowText = fullVal.length > 2000 ? [fullVal substringToIndex:2000] : fullVal;
            }
        }

        if (windowText.length > 0 && charRange.location >= winStart) {
            NSInteger localOffset = charRange.location - winStart;
            NSInteger localLen = charRange.length > 0 ? charRange.length : 1;

            CGRect rangeBounds = CGRectZero;
            bool hasBounds = lt_ax_bounds_for_range(hitElem, charRange, &rangeBounds);
            if (!hasBounds) {
                hasBounds = lt_ax_frame(hitElem, &rangeBounds);
            }

            // Check if leaf text is short/fragmented -> climb 1~2 parent paragraph nodes for full sentence!
            NSDictionary *paraAgg = nil;
            if (windowText.length < 240 && winStart == 0) {
                paraAgg = lt_try_aggregate_parent_paragraph(hitElem, localOffset);
            }

            NSMutableDictionary *resp = [@{
                @"status": @"rangeResolved",
                @"role": role,
                @"windowText": windowText,
                @"windowStart": @(winStart),
                @"elementTotal": totalLen >= 0 ? @(totalLen) : [NSNull null],
                @"hitOffset": @(localOffset),
                @"hitLength": @(localLen),
                @"appId": appId ?: [NSNull null]
            } mutableCopy];

            if (paraAgg) {
                resp[@"paragraphText"] = paraAgg[@"paragraphText"];
                resp[@"paragraphHitOffset"] = paraAgg[@"paragraphHitOffset"];
            }
            if (hasBounds) {
                resp[@"rect"] = @{
                    @"x": @(rangeBounds.origin.x),
                    @"y": @(rangeBounds.origin.y),
                    @"width": @(rangeBounds.size.width),
                    @"height": @(rangeBounds.size.height)
                };
            }

            CFRelease(hitElem);
            return lt_json_encode(resp);
        }
    }

    // 3. Bounded short control / link / heading / button label fallback
    CGRect elemRect = CGRectZero;
    bool hasElemRect = lt_ax_frame(hitElem, &elemRect);
    NSString *labelText = lt_best_element_text(hitElem, role);
    NSString *trimmedLabel = [labelText stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    CFRelease(hitElem);

    if (trimmedLabel.length > 0 && trimmedLabel.length <= 180 &&
        (!hasElemRect || (elemRect.size.height <= 96.0 && CGRectContainsPoint(CGRectInset(elemRect, -4.0, -4.0), pt)))) {
        NSMutableDictionary *resp = [@{
            @"status": @"labelResolved",
            @"role": role,
            @"label": trimmedLabel,
            @"appId": appId ?: [NSNull null]
        } mutableCopy];
        if (hasElemRect) {
            resp[@"rect"] = @{
                @"x": @(elemRect.origin.x),
                @"y": @(elemRect.origin.y),
                @"width": @(elemRect.size.width),
                @"height": @(elemRect.size.height)
            };
        }
        return lt_json_encode(resp);
    }

    return lt_json_encode(@{
        @"status": @"notSupported",
        @"appId": appId ?: [NSNull null]
    });
}

// ============================================================================
// Local Offline Apple Vision OCR (Zero Cooldown, Language Correction On, No 1px Drop)
// ============================================================================

static NSArray<NSDictionary *> *lt_run_vision_ocr(CGImageRef cgImage, CGRect screenRegion) {
    if (!cgImage) return @[];

    VNRecognizeTextRequest *request = [[VNRecognizeTextRequest alloc] init];
    request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    request.usesLanguageCorrection = YES;
    request.recognitionLanguages = @[@"en-US", @"zh-Hans", @"zh-Hant"];

    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:cgImage options:@{}];
    NSError *err = nil;
    if (![handler performRequests:@[request] error:&err] || err) {
        return @[];
    }

    NSMutableArray<NSDictionary *> *outLines = [NSMutableArray array];
    for (VNRecognizedTextObservation *obs in (request.results ?: @[])) {
        VNRecognizedText *cand = [[obs topCandidates:1] firstObject];
        if (!cand || cand.string.length == 0) continue;

        NSString *lineStr = cand.string;
        float conf = cand.confidence;

        NSMutableArray<NSDictionary *> *rawWords = [NSMutableArray array];
        [lineStr enumerateSubstringsInRange:NSMakeRange(0, lineStr.length)
                                    options:NSStringEnumerationByWords
                                 usingBlock:^(NSString *substring, NSRange wordRange, NSRange enclosingRange, BOOL *stop) {
            (void)enclosingRange;
            (void)stop;
            if (!substring || substring.length == 0) return;
            NSError *boxErr = nil;
            VNRectangleObservation *boxObs = [cand boundingBoxForRange:wordRange error:&boxErr];
            CGRect normBox = (boxObs && !boxErr) ? boxObs.boundingBox : obs.boundingBox;

            // Vision normalized rect uses bottom-left origin (0..1); convert to top-left screen coordinates!
            double wx = screenRegion.origin.x + normBox.origin.x * screenRegion.size.width;
            double wy = screenRegion.origin.y + (1.0 - normBox.origin.y - normBox.size.height) * screenRegion.size.height;
            double ww = normBox.size.width * screenRegion.size.width;
            double wh = normBox.size.height * screenRegion.size.height;

            [rawWords addObject:@{
                @"text": substring,
                @"rangeLoc": @(wordRange.location),
                @"rangeEnd": @(NSMaxRange(wordRange)),
                @"x": @(wx),
                @"y": @(wy),
                @"w": @(ww),
                @"h": @(wh)
            }];
        }];

        if (rawWords.count == 0) continue;

        NSMutableArray<NSDictionary *> *finalWords = [NSMutableArray arrayWithCapacity:rawWords.count];
        for (NSUInteger idx = 0; idx < rawWords.count; idx++) {
            NSDictionary *item = rawWords[idx];
            NSUInteger endLoc = [item[@"rangeEnd"] unsignedIntegerValue];
            NSUInteger nextStart = (idx + 1 < rawWords.count)
                ? [rawWords[idx + 1][@"rangeLoc"] unsignedIntegerValue]
                : lineStr.length;

            NSString *trailing = @"";
            if (nextStart > endLoc && nextStart <= lineStr.length) {
                trailing = [[lineStr substringWithRange:NSMakeRange(endLoc, nextStart - endLoc)]
                    stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
            }

            [finalWords addObject:@{
                @"text": item[@"text"],
                @"trailing": trailing ?: @"",
                @"confidence": @(conf),
                @"rect": @{
                    @"x": item[@"x"],
                    @"y": item[@"y"],
                    @"width": item[@"w"],
                    @"height": item[@"h"]
                }
            }];
        }

        [outLines addObject:@{
            @"text": lineStr,
            @"words": finalWords
        }];
    }

    return outLines;
}

static double lt_display_scale_for_point(CGPoint pt) {
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) == kCGErrorSuccess && count > 0) {
        CGDirectDisplayID displays[32];
        uint32_t cap = count > 32 ? 32 : count;
        if (CGGetActiveDisplayList(cap, displays, &cap) == kCGErrorSuccess) {
            for (uint32_t i = 0; i < cap; i++) {
                CGRect b = CGDisplayBounds(displays[i]);
                if (CGRectContainsPoint(b, pt) && b.size.width > 0) {
                    return (double)CGDisplayPixelsWide(displays[i]) / b.size.width;
                }
            }
        }
    }
    return 2.0;
}

static CGImageRef lt_sck_capture_window_rect(uint32_t targetWinId, CGRect screenClippedRect) {
    __block SCShareableContent *shareable = nil;
    dispatch_semaphore_t semContent = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *content, NSError *error) {
        if (!error) shareable = content;
        dispatch_semaphore_signal(semContent);
    }];
    dispatch_semaphore_wait(semContent, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)));
    if (!shareable) return NULL;

    SCWindow *targetWin = nil;
    for (SCWindow *w in shareable.windows) {
        if (w.windowID == targetWinId) {
            targetWin = w;
            break;
        }
    }
    if (!targetWin) return NULL;

    CGRect localRect = CGRectMake(
        screenClippedRect.origin.x - targetWin.frame.origin.x,
        screenClippedRect.origin.y - targetWin.frame.origin.y,
        screenClippedRect.size.width,
        screenClippedRect.size.height
    );
    if (localRect.size.width < 4.0 || localRect.size.height < 4.0) return NULL;

    double scale = lt_display_scale_for_point(screenClippedRect.origin);
    SCContentFilter *filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:targetWin];
    SCStreamConfiguration *cfg = [[SCStreamConfiguration alloc] init];
    cfg.sourceRect = localRect;
    cfg.width = (size_t)ceil(localRect.size.width * scale);
    cfg.height = (size_t)ceil(localRect.size.height * scale);
    cfg.showsCursor = NO;
    cfg.capturesAudio = NO;

    __block CGImageRef captured = NULL;
    dispatch_semaphore_t semShot = dispatch_semaphore_create(0);
    [SCScreenshotManager captureImageWithFilter:filter
                                  configuration:cfg
                              completionHandler:^(CGImageRef sampleBuffer, NSError *error) {
        if (!error && sampleBuffer) {
            captured = CGImageRetain(sampleBuffer);
        }
        dispatch_semaphore_signal(semShot);
    }];
    dispatch_semaphore_wait(semShot, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)));
    return captured;
}

static CGImageRef lt_sck_capture_display_region(CGRect screenRegion) {
    __block SCShareableContent *shareable = nil;
    dispatch_semaphore_t semContent = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *content, NSError *error) {
        if (!error) shareable = content;
        dispatch_semaphore_signal(semContent);
    }];
    dispatch_semaphore_wait(semContent, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)));
    if (!shareable || shareable.displays.count == 0) return NULL;

    CGPoint center = CGPointMake(CGRectGetMidX(screenRegion), CGRectGetMidY(screenRegion));
    SCDisplay *targetDisplay = shareable.displays.firstObject;
    for (SCDisplay *d in shareable.displays) {
        if (CGRectContainsPoint(d.frame, center)) {
            targetDisplay = d;
            break;
        }
    }

    pid_t selfPid = getpid();
    NSMutableArray<SCWindow *> *excluded = [NSMutableArray array];
    for (SCWindow *w in shareable.windows) {
        if (w.owningApplication.processID == selfPid || (g_PopupWindowNumber != 0 && w.windowID == g_PopupWindowNumber)) {
            [excluded addObject:w];
        }
    }

    CGRect clipped = CGRectIntersection(screenRegion, targetDisplay.frame);
    if (CGRectIsNull(clipped) || clipped.size.width < 4.0 || clipped.size.height < 4.0) return NULL;

    CGRect localRect = CGRectMake(
        clipped.origin.x - targetDisplay.frame.origin.x,
        clipped.origin.y - targetDisplay.frame.origin.y,
        clipped.size.width,
        clipped.size.height
    );
    double scale = lt_display_scale_for_point(center);

    SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:targetDisplay excludingWindows:excluded];
    SCStreamConfiguration *cfg = [[SCStreamConfiguration alloc] init];
    cfg.sourceRect = localRect;
    cfg.width = (size_t)ceil(localRect.size.width * scale);
    cfg.height = (size_t)ceil(localRect.size.height * scale);
    cfg.showsCursor = NO;
    cfg.capturesAudio = NO;

    __block CGImageRef captured = NULL;
    dispatch_semaphore_t semShot = dispatch_semaphore_create(0);
    [SCScreenshotManager captureImageWithFilter:filter
                                  configuration:cfg
                              completionHandler:^(CGImageRef sampleBuffer, NSError *error) {
        if (!error && sampleBuffer) {
            captured = CGImageRetain(sampleBuffer);
        }
        dispatch_semaphore_signal(semShot);
    }];
    dispatch_semaphore_wait(semShot, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)));
    return captured;
}

char *lt_macos_ocr_at_point_json(double x, double y, bool sentence_mode) {
    if (!CGPreflightScreenCaptureAccess()) {
        return lt_json_encode(@{ @"status": @"permissionDenied" });
    }

    CGPoint pt = CGPointMake(x, y);
    LTWindowHit winHit = lt_topmost_window_at(pt);
    if (!winHit.valid) {
        return lt_json_encode(@{ @"status": @"noText" });
    }

    double halfW = sentence_mode ? 540.0 : 320.0;
    double halfH = sentence_mode ? 180.0 : 90.0;
    CGRect desired = CGRectMake(pt.x - halfW, pt.y - halfH, halfW * 2.0, halfH * 2.0);
    CGRect clipped = CGRectIntersection(desired, winHit.bounds);
    if (CGRectIsNull(clipped) || clipped.size.width < 8 || clipped.size.height < 8) {
        return lt_json_encode(@{ @"status": @"noText" });
    }

    CGImageRef img = lt_sck_capture_window_rect(winHit.window_id, clipped);
    if (!img) {
        return lt_json_encode(@{ @"status": @"noText" });
    }
    if (CGImageGetWidth(img) < 4 || CGImageGetHeight(img) < 4) {
        CFRelease(img);
        return lt_json_encode(@{ @"status": @"noText" });
    }

    NSArray<NSDictionary *> *lines = lt_run_vision_ocr(img, clipped);
    CFRelease(img);

    NSString *appId = lt_bundle_id_for_pid(winHit.pid);
    return lt_json_encode(@{
        @"status": @"ok",
        @"lines": lines,
        @"appId": appId ?: [NSNull null]
    });
}

char *lt_macos_ocr_region_json(double x, double y, double w, double h) {
    if (!CGPreflightScreenCaptureAccess()) {
        return lt_json_encode(@{ @"status": @"permissionDenied" });
    }
    CGRect region = CGRectMake(x, y, w, h);
    if (region.size.width < 6 || region.size.height < 6) {
        return lt_json_encode(@{ @"status": @"noText" });
    }

    CGImageRef img = lt_sck_capture_display_region(region);
    if (!img) {
        return lt_json_encode(@{ @"status": @"noText" });
    }

    NSArray<NSDictionary *> *lines = lt_run_vision_ocr(img, region);
    CFRelease(img);

    return lt_json_encode(@{
        @"status": @"ok",
        @"lines": lines
    });
}

// ============================================================================
// Non-Activating NSPanel & Mouse Pass-Through Window Controls
// ============================================================================

static void lt_apply_nonactivating_panel_class(NSWindow *win) {
    if (!win) return;
    Class curCls = object_getClass(win);
    if ([win isKindOfClass:[LTNonActivatingPanel class]]) return;

    // Safely convert or dynamically subclass `curCls` so `_preventsActivation` & `canBecomeKeyWindow = NO` apply
    if (class_getInstanceSize(curCls) == class_getInstanceSize([LTNonActivatingPanel class])) {
        object_setClass(win, [LTNonActivatingPanel class]);
    } else {
        NSString *subName = [NSString stringWithFormat:@"LTNonActivating_%s", class_getName(curCls)];
        Class subCls = NSClassFromString(subName);
        if (!subCls) {
            subCls = objc_allocateClassPair(curCls, [subName UTF8String], 0);
            if (subCls) {
                IMP noImp = imp_implementationWithBlock(^BOOL(id _self) { (void)_self; return NO; });
                IMP yesImp = imp_implementationWithBlock(^BOOL(id _self) { (void)_self; return YES; });
                class_addMethod(subCls, @selector(canBecomeKeyWindow), noImp, "c@:");
                class_addMethod(subCls, @selector(canBecomeMainWindow), noImp, "c@:");
                class_addMethod(subCls, NSSelectorFromString(@"_preventsActivation"), yesImp, "c@:");
                objc_registerClassPair(subCls);
            }
        }
        if (subCls) {
            object_setClass(win, subCls);
        }
    }
}

void lt_macos_configure_popup_window(void *ns_window_ptr) {
    if (!ns_window_ptr) return;
    NSWindow *win = (__bridge NSWindow *)ns_window_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
        lt_apply_nonactivating_panel_class(win);
        [win setStyleMask:([win styleMask] | NSWindowStyleMaskNonactivatingPanel | NSWindowStyleMaskBorderless)];
        if ([win isKindOfClass:[NSPanel class]]) {
            [(NSPanel *)win setFloatingPanel:YES];
            [(NSPanel *)win setBecomesKeyOnlyIfNeeded:YES];
        }
        [win setLevel:NSPopUpMenuWindowLevel + 1];
        [win setHidesOnDeactivate:NO];
        [win setOpaque:NO];
        [win setBackgroundColor:[NSColor clearColor]];
        [win setHasShadow:NO];
        [win setAnimationBehavior:NSWindowAnimationBehaviorNone];
        [win setIgnoresMouseEvents:YES];
        [win setCollectionBehavior:(NSWindowCollectionBehaviorCanJoinAllSpaces
                                  | NSWindowCollectionBehaviorFullScreenAuxiliary
                                  | NSWindowCollectionBehaviorIgnoresCycle
                                  | NSWindowCollectionBehaviorTransient)];
        g_PopupWindowNumber = (uint32_t)[win windowNumber];
    });
}

void lt_macos_present_popup_window(void *ns_window_ptr, double x, double y, double w, double h, bool interactive) {
    if (!ns_window_ptr) return;
    NSWindow *win = (__bridge NSWindow *)ns_window_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
        NSRect appKitRect = lt_topleft_rect_to_appkit(x, y, w, h);
        [win setFrame:appKitRect display:YES animate:NO];
        [win setIgnoresMouseEvents:!interactive];
        [win orderFrontRegardless];
        g_PopupWindowNumber = (uint32_t)[win windowNumber];
    });
}

void lt_macos_set_popup_interactive(void *ns_window_ptr, bool interactive) {
    if (!ns_window_ptr) return;
    NSWindow *win = (__bridge NSWindow *)ns_window_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
        [win setIgnoresMouseEvents:!interactive];
    });
}

void lt_macos_hide_popup_window(void *ns_window_ptr) {
    if (!ns_window_ptr) return;
    NSWindow *win = (__bridge NSWindow *)ns_window_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
        [win setIgnoresMouseEvents:YES];
        [win orderOut:nil];
    });
}
