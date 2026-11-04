(function () {
  var ua = navigator.userAgent || "";
  var ios = /iP(hone|ad|od)/.test(ua);
  var android = /Android/i.test(ua);
  var crios = /CriOS/i.test(ua);
  var chromeAndroid = android && /Chrome|Chromium/i.test(ua) && !/EdgA|OPR|SamsungBrowser/i.test(ua);
  var safari = ios && /WebKit/i.test(ua) && !crios && !/FxiOS|EdgiOS/.test(ua);
  document.documentElement.setAttribute(
    "data-browser",
    safari ? "safari" : crios || chromeAndroid ? "chrome" : "other",
  );
  if (ios) {
    document.documentElement.classList.add("is-frost-deferred");
  }
  // iOS exposes the home-screen state as `navigator.standalone`; the CSS media
  // query alone misses some WebKit builds, and the class lets the stylesheet
  // drop the glass pass before the first paint.
  try {
    var mq = window.matchMedia;
    var standalone =
      window.navigator.standalone === true ||
      (mq &&
        (mq("(display-mode: standalone)").matches ||
          mq("(display-mode: fullscreen)").matches ||
          mq("(display-mode: minimal-ui)").matches));
    if (standalone) document.documentElement.classList.add("is-standalone");
  } catch (e) {
    /* ignore */
  }
  if (safari) {
    var meta = document.querySelector('meta[name="viewport"]');
    if (meta) {
      meta.setAttribute(
        "content",
        "width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover, shrink-to-fit=no",
      );
    }
  }

  try {
    if (localStorage.getItem("album-theme") === "dark") {
      document.documentElement.classList.add("dark");
      var themeColor = document.querySelector('meta[name="theme-color"]');
      if (themeColor) themeColor.setAttribute("content", "#0c1418");
    }
  } catch (e) {
    /* ignore quota / private mode */
  }

  try {
    var raw = Number(localStorage.getItem("album-ui-scale"));
    if (Number.isFinite(raw) && raw !== 100 && raw !== 1) {
      var pct = raw > 0 && raw <= 2 ? Math.round(raw * 100) : Math.round(raw);
      pct = Math.min(150, Math.max(75, pct));
      if (pct !== 100) {
        var z = pct / 100;
        var rootScale = document.documentElement;
        rootScale.style.setProperty("--ui-scale", String(z));
        rootScale.style.setProperty("--ui-zoom-inv", "1");
        rootScale.classList.add("is-ui-scaled");
      }
    }
  } catch (e) {
    /* ignore quota / private mode */
  }

  function readPx(value) {
    var n = parseFloat(value);
    return Number.isFinite(n) && n > 0 ? n : 0;
  }

  function lvBox() {
    var el = document.createElement("div");
    el.style.cssText =
      "position:absolute;left:0;top:0;width:100lvw;height:100lvh;visibility:hidden;pointer-events:none;zoom:1";
    document.documentElement.appendChild(el);
    var box = { w: el.offsetWidth || 0, h: el.offsetHeight || 0 };
    el.remove();
    return box;
  }

  function measureInsets() {
    var probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;inset:0;visibility:hidden;pointer-events:none;" +
      "padding:env(safe-area-inset-top,0px) env(safe-area-inset-right,0px) env(safe-area-inset-bottom,0px) env(safe-area-inset-left,0px)";
    document.documentElement.appendChild(probe);
    var cs = getComputedStyle(probe);
    var insets = {
      top: readPx(cs.paddingTop),
      right: readPx(cs.paddingRight),
      bottom: readPx(cs.paddingBottom),
      left: readPx(cs.paddingLeft),
    };
    probe.remove();
    return insets;
  }

  function coverRect() {
    var iw = window.innerWidth || 0;
    var ih = window.innerHeight || 0;
    var lv = lvBox();
    var w = Math.max(iw, lv.w);
    var h = Math.max(lv.h, ih);
    var insets = measureInsets();
    var bottom = insets.bottom;
    var portrait = (window.innerHeight || 0) >= (window.innerWidth || 0);
    if (ios && portrait && insets.top >= 44 && bottom < 8) {
      bottom = 34;
    }
    return {
      w: Math.max(1, Math.round(w)),
      h: Math.max(1, Math.round(h)),
      top: insets.top,
      right: insets.right,
      bottom: bottom,
      left: insets.left,
    };
  }

  function paintCover(splash) {
    var rect = coverRect();
    if (splash && splash.isConnected) {
      splash.style.top = "0px";
      splash.style.left = "0px";
      splash.style.right = "auto";
      splash.style.bottom = "auto";
      splash.style.width = rect.w + "px";
      splash.style.height = rect.h + "px";
    }
    var root = document.documentElement;
    root.style.setProperty("--app-height", rect.h + "px");
    root.style.setProperty("--app-top", "0px");
    root.style.setProperty("--safe-top-js", rect.top + "px");
    root.style.setProperty("--safe-right-js", rect.right + "px");
    root.style.setProperty("--safe-bottom-js", rect.bottom + "px");
    root.style.setProperty("--safe-left-js", rect.left + "px");
    return rect;
  }

  function initSplash() {
    var splash = document.getElementById("boot-splash");
    if (!splash) return;

    var stackDone = false;
    var appReady = false;
    var hiding = false;

    function kickLayers() {
      document.documentElement.classList.remove("is-frost-deferred");
    }

    function hide() {
      if (hiding || !stackDone || !appReady) return;
      hiding = true;
      paintCover(splash);
      // Snap to the clipped mobile layout while splash still covers.
      // Frost stays off until that layout is committed: the first
      // backdrop-filter snapshot must not be built against
      // html.is-booting { overflow: visible; height: auto }.
      document.documentElement.classList.remove("is-booting");
      void document.documentElement.offsetHeight;
      function startLeave() {
        splash.classList.add("is-leaving");
      }
      if (ios) {
        requestAnimationFrame(function () {
          requestAnimationFrame(function () {
            kickLayers();
            requestAnimationFrame(function () {
              requestAnimationFrame(startLeave);
            });
          });
        });
      } else {
        document.documentElement.classList.remove("is-frost-deferred");
        startLeave();
      }
      var done = false;
      function finish() {
        if (done) return;
        done = true;
        splash.remove();
        document.documentElement.classList.remove("is-booting");
        document.documentElement.classList.remove("is-frost-deferred");
        window.removeEventListener("resize", onViewport);
        window.removeEventListener("orientationchange", onViewport);
        window.visualViewport && window.visualViewport.removeEventListener("resize", onViewport);
        window.visualViewport && window.visualViewport.removeEventListener("scroll", onViewport);
        window.dispatchEvent(new Event("lumeno:booted"));
      }
      splash.addEventListener("transitionend", finish);
      setTimeout(finish, 420);
    }

    paintCover(splash);
    requestAnimationFrame(function () {
      paintCover(splash);
      splash.classList.add("is-entering");
    });
    function onViewport() {
      if (hiding) return;
      paintCover(splash);
    }
    window.addEventListener("resize", onViewport);
    window.addEventListener("orientationchange", onViewport);
    window.visualViewport && window.visualViewport.addEventListener("resize", onViewport);
    window.visualViewport && window.visualViewport.addEventListener("scroll", onViewport);
    setTimeout(onViewport, 120);
    setTimeout(onViewport, 360);

    setTimeout(function () {
      stackDone = true;
      hide();
    }, 1080);

    window.addEventListener("lumeno:ready", function () {
      appReady = true;
      hide();
    });
    setTimeout(function () {
      appReady = true;
      hide();
    }, 10000);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initSplash);
  } else {
    initSplash();
  }
})();
