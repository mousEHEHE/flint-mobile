// Adapted from gpui-mobile's example. The iOS 27 SDK requires scenes, and iOS can
// replace a scene with a new one, so gpui's window is kept in the app delegate.

#import <UIKit/UIKit.h>
#import "gpui_ios.h"

@interface FlintAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, assign) void *gpuiWindow;
@property (nonatomic, strong) CADisplayLink *displayLink;
- (UIWindow *)windowForScene:(UIWindowScene *)scene;
- (void)startDisplayLink;
- (void)stopDisplayLink;
@end

@interface FlintSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

static void FlintResumeFrames(void *context);

static FlintAppDelegate *FlintApp(void) {
    return (FlintAppDelegate *)UIApplication.sharedApplication.delegate;
}

@implementation FlintAppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    return YES;
}

- (UIWindow *)windowForScene:(UIWindowScene *)scene {
    if (!self.gpuiWindow) {
        gpui_ios_register_app();
        gpui_ios_run_demo();
        self.gpuiWindow = gpui_ios_get_window();
        if (!self.gpuiWindow) {
            NSLog(@"flint: gpui opened no window");
            return nil;
        }
    }
    // gpui-mobile puts its window on the first scene; a replacement scene takes it over.
    UIViewController *controller = (__bridge UIViewController *)gpui_ios_view_controller(self.gpuiWindow);
    UIWindow *window = controller.view.window;
    if (!window) {
        NSLog(@"flint: gpui's view has no window");
        return nil;
    }
    if (window.windowScene != scene) {
        NSLog(@"flint: moving gpui's window to the scene");
        window.windowScene = scene;
    }
    [window makeKeyAndVisible];
    [self startDisplayLink];
    return window;
}

// The link pauses when gpui wants no more frames and the waker resumes it, so idle costs no CPU.
- (void)startDisplayLink {
    if (self.displayLink || !self.gpuiWindow) {
        return;
    }
    self.displayLink = [CADisplayLink displayLinkWithTarget:self selector:@selector(renderFrame)];
    [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
    gpui_ios_set_frame_waker(self.gpuiWindow, FlintResumeFrames, (__bridge void *)self);
}

- (void)stopDisplayLink {
    [self.displayLink invalidate];
    self.displayLink = nil;
}

- (void)renderFrame {
    if (self.gpuiWindow && !gpui_ios_request_frame(self.gpuiWindow)) {
        self.displayLink.paused = YES;
    }
}

static void FlintResumeFrames(void *context) {
    FlintAppDelegate *delegate = (__bridge FlintAppDelegate *)context;
    delegate.displayLink.paused = NO;
}

- (void)applicationWillTerminate:(UIApplication *)application {
    [self stopDisplayLink];
    gpui_ios_will_terminate(NULL);
}

@end

@implementation FlintSceneDelegate

- (void)scene:(UIScene *)scene willConnectToSession:(UISceneSession *)session options:(UISceneConnectionOptions *)connectionOptions {
    if (![scene isKindOfClass:[UIWindowScene class]]) {
        return;
    }
    self.window = [FlintApp() windowForScene:(UIWindowScene *)scene];
    // A URL that launched the app arrives here, not in scene:openURLContexts:.
    [self openURLContexts:connectionOptions.URLContexts];
}

- (void)sceneWillEnterForeground:(UIScene *)scene {
    gpui_ios_will_enter_foreground(NULL);
    [FlintApp() startDisplayLink];
}

- (void)sceneDidBecomeActive:(UIScene *)scene {
    gpui_ios_did_become_active(NULL);
}

- (void)sceneWillResignActive:(UIScene *)scene {
    gpui_ios_will_resign_active(NULL);
}

- (void)sceneDidEnterBackground:(UIScene *)scene {
    gpui_ios_did_enter_background(NULL);
    [FlintApp() stopDisplayLink];
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
    [self openURLContexts:URLContexts];
}

- (void)openURLContexts:(NSSet<UIOpenURLContext *> *)contexts {
    for (UIOpenURLContext *context in contexts) {
        gpui_ios_handle_open_url((__bridge void *)context.URL.absoluteString);
    }
}

@end

int main(int argc, char *argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass([FlintAppDelegate class]));
    }
}
