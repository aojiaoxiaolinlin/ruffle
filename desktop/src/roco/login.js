// Runs in every frame. Only game result pages inspect FlashVars; QQ login
// frames are untouched. No password fields or cookie values are read.
(() => {
    "use strict";
    const resultOrigin = origin => origin === "https://17roco.qq.com"
        || origin === "https://web2.17roco.qq.com";
    const official = location.protocol === "https:"
        && (location.hostname === "17roco.qq.com"
            || location.hostname === "web2.17roco.qq.com");
    const resultPage = official && (
        (location.hostname === "17roco.qq.com"
            && ["/main.html", "/logintarget.html"].includes(location.pathname))
        || (location.hostname === "web2.17roco.qq.com"
            && ["/fcgi-bin/login2", "/fcgi-bin/login3"].includes(location.pathname))
    );
    let sent = false;
    let nightSent = false;
    const sendNight = () => {
        if (nightSent) return;
        nightSent = true;
        if (window === window.top) {
            window.ipc.postMessage(JSON.stringify({type: "ruffle-roco-night"}));
        } else {
            // WebView2's main WebMessageReceived does not receive frame IPC.
            for (const origin of ["https://17roco.qq.com", "https://web2.17roco.qq.com"]) {
                window.top.postMessage({type: "ruffle-roco-night"}, origin);
            }
        }
    };
    const findNight = () => {
        if (nightSent || !official) return;
        let html = "";
        try {
            html = document.documentElement ? document.documentElement.innerHTML : "";
        } catch (_error) {}
        if (html.includes("night.swf")) {
            sendNight();
            return;
        }
        for (const node of document.querySelectorAll("param,embed,object")) {
            const value = node.getAttribute("src")
                || node.getAttribute("data")
                || node.getAttribute("value") || "";
            if (value.includes("night.swf")) {
                sendNight();
                return;
            }
        }
    };
    const deliver = raw => {
        if (sent || typeof raw !== "string" || raw.length > 131072) return;
        const values = new URLSearchParams(raw);
        if (!["angel_uin", "angel_key", "skey", "pskey"].every(key => {
            const value = values.get(key);
            return value && value.trim() && !value.includes("{{") && value.length <= 16384;
        })) return;
        sent = true;
        if (window === window.top) {
            window.ipc.postMessage(JSON.stringify(raw));
        } else {
            // WebView2's main WebMessageReceived does not receive frame IPC.
            // Forward to the trusted top document, which validates the origin.
            for (const origin of ["https://17roco.qq.com", "https://web2.17roco.qq.com"]) {
                window.top.postMessage({type: "ruffle-roco-flashvars", raw}, origin);
            }
        }
    };
    if (window === window.top && official) {
        window.addEventListener("message", event => {
            if (!resultOrigin(event.origin) || !event.data) return;
            if (event.data.type === "ruffle-roco-flashvars") {
                deliver(event.data.raw);
            } else if (event.data.type === "ruffle-roco-night") {
                sendNight();
            }
        });
    }
    if (official) {
        new MutationObserver(findNight).observe(document, {childList: true, subtree: true, attributes: true});
        document.addEventListener("DOMContentLoaded", findNight);
        window.addEventListener("load", findNight);
        findNight();
    }
    if (!resultPage) return;
    const read = () => {
        if (sent) return;
        for (const node of document.querySelectorAll("param,embed")) {
            const raw = node.tagName.toLowerCase() === "param"
                ? ((node.getAttribute("name") || "").toLowerCase() === "flashvars"
                    ? node.getAttribute("value") : null)
                : node.getAttribute("flashvars");
            if (raw) deliver(raw);
        }
    };
    new MutationObserver(read).observe(document, {childList: true, subtree: true, attributes: true});
    document.addEventListener("DOMContentLoaded", read);
    read();
})();
