// Run with: node --test desktop/tests/roco-login.test.cjs
const {test} = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');
const script = fs.readFileSync(path.join(__dirname, '../src/roco/login.js'), 'utf8');
const vars = 'angel_uin=123&angel_key=a%2Bb%26c%3D&skey=s&pskey=p';

function page(url, {frame = false, raw = vars, nodes: initialNodes} = {}) {
    const listeners = {};
    const messages = [];
    let mutate;
    let nodes = initialNodes
        || [{tagName: 'PARAM', getAttribute: key => ({name: 'FlashVars', value: raw})[key]}];
    const window = {
        ipc: {postMessage: body => messages.push(JSON.parse(body))},
        addEventListener: (name, fn) => { listeners[name] = fn; },
    };
    window.top = frame ? {postMessage: (body, origin) => {
        // Model a top-level official login document. Normalize the object so
        // assertions compare values rather than VM-realm prototypes.
        if (origin === 'https://17roco.qq.com') messages.push(JSON.parse(JSON.stringify(body)));
    }} : window;
    const document = {
        querySelectorAll: () => nodes,
        addEventListener: (name, fn) => { listeners[name] = fn; },
    };
    vm.runInNewContext(script, {
        window, document, location: new URL(url), URLSearchParams,
        MutationObserver: class { constructor(fn) { mutate = fn; } observe() {} },
    });
    return {messages, listeners, mutate: () => mutate?.(), setNodes: next => { nodes = next; }};
}

test('extracts decoded DOM FlashVars once, including markup added after load', () => {
    const p = page('https://web2.17roco.qq.com/fcgi-bin/login3', {raw: ''});
    assert.equal(p.messages.length, 0);
    p.setNodes([{tagName: 'EMBED', getAttribute: key => key === 'flashvars' ? vars : null}]);
    p.mutate();
    p.mutate();
    assert.deepEqual(p.messages, [vars]);
});

test('opens the official night notice instead of waiting for FlashVars', () => {
    const night = {
        tagName: 'EMBED',
        getAttribute: key => key === 'src' ? '//res.17roco.qq.com/swf/night.swf' : null,
    };
    const p = page('https://web2.17roco.qq.com/fcgi-bin/login2', {nodes: [night]});
    assert.deepEqual(p.messages, [{type: 'ruffle-roco-night'}]);
});

test('night notice is forwarded from a login frame', () => {
    const night = {
        tagName: 'EMBED',
        getAttribute: key => key === 'src' ? '//res.17roco.qq.com/swf/night.swf' : null,
    };
    const p = page('https://web2.17roco.qq.com/fcgi-bin/login2', {frame: true, nodes: [night]});
    assert.deepEqual(p.messages, [{type: 'ruffle-roco-night'}]);
});

test('result iframe forwards to top rather than inaccessible native frame IPC', () => {
    const p = page('https://web2.17roco.qq.com/fcgi-bin/login3', {frame: true});
    assert.equal(p.messages.length, 1);
    assert.equal(p.messages[0].type, 'ruffle-roco-flashvars');
    assert.equal(p.messages[0].raw, vars);
});

test('top login accepts only official result origins and ignores duplicate completion', () => {
    const p = page('https://17roco.qq.com/login.html');
    const data = {type: 'ruffle-roco-flashvars', raw: vars};
    p.listeners.message({origin: 'https://17roco.qq.com.evil.test', data});
    p.listeners.message({origin: 'https://graph.qq.com', data});
    assert.equal(p.messages.length, 0);
    p.listeners.message({origin: 'https://web2.17roco.qq.com', data});
    p.listeners.message({origin: 'https://web2.17roco.qq.com', data});
    assert.deepEqual(p.messages, [vars]);
});

test('does not inspect QQ login pages or accept missing/template credentials', () => {
    assert.equal(page('https://graph.qq.com/oauth2.0/authorize').messages.length, 0);
    assert.equal(page('http://web2.17roco.qq.com/fcgi-bin/login3').messages.length, 0);
    for (const raw of ['angel_key=k', vars.replace('pskey=p', 'pskey={{pskey}}')]) {
        assert.equal(page('https://17roco.qq.com/main.html', {raw}).messages.length, 0);
    }
});
