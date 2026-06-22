(function () {
    function addExpandButtons() {
        document.querySelectorAll('.live-demo-container').forEach(function (container) {
            if (container.querySelector('.live-demo-expand-btn')) return;

            var btn = document.createElement('button');
            btn.className = 'live-demo-expand-btn';
            btn.textContent = 'Expand';
            btn.setAttribute('aria-label', 'Expand demo to fill window');

            btn.addEventListener('click', function () {
                var expanded = container.classList.toggle('live-demo-fullscreen');
                btn.textContent = expanded ? 'Collapse' : 'Expand';
                btn.setAttribute('aria-label', expanded
                    ? 'Collapse demo to inline'
                    : 'Expand demo to fill window');
                document.body.style.overflow = expanded ? 'hidden' : '';
            });

            container.appendChild(btn);
        });
    }

    // Collapse on Escape
    document.addEventListener('keydown', function (e) {
        if (e.key !== 'Escape') return;
        document.querySelectorAll('.live-demo-container.live-demo-fullscreen').forEach(function (container) {
            container.classList.remove('live-demo-fullscreen');
            var btn = container.querySelector('.live-demo-expand-btn');
            if (btn) {
                btn.textContent = 'Expand';
                btn.setAttribute('aria-label', 'Expand demo to fill window');
            }
            document.body.style.overflow = '';
        });
    });

    // Trim the theme picker to just Light + Dark (+ Auto). The guide
    // only ships two themes (see custom.css); hide the rest and rename
    // "Coal" to the friendlier "Dark".
    function pruneThemes() {
        ['rust', 'navy', 'ayu'].forEach(function (id) {
            var btn = document.getElementById('mdbook-theme-' + id);
            if (!btn) return;
            var li = btn.closest('li');
            (li || btn).remove();
        });
        var coal = document.getElementById('mdbook-theme-coal');
        if (coal) coal.textContent = 'Dark';
    }

    // The Rust API links (sidebar entry + the reference-page link) point at the
    // generated rustdoc, which lives outside the mdBook chrome — open it in a
    // new tab. mdBook hardcodes target="_parent" on sidebar links, and the
    // sidebar is injected asynchronously, so we fix it up here (also re-run by
    // the observer below).
    function openApiLinksInNewTab() {
        document.querySelectorAll('a[href*="api/bovista/"]').forEach(function (a) {
            a.target = '_blank';
            a.rel = 'noopener noreferrer';
        });
    }

    function init() {
        addExpandButtons();
        pruneThemes();
        openApiLinksInNewTab();
    }

    // Initial run
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }

    // Re-run on mdBook SPA navigation (content div is replaced on page change)
    var observer = new MutationObserver(function (mutations) {
        for (var i = 0; i < mutations.length; i++) {
            if (mutations[i].addedNodes.length) {
                addExpandButtons();
                openApiLinksInNewTab();
                break;
            }
        }
    });
    observer.observe(document.body, { childList: true, subtree: true });
}());
