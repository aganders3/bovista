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

    // Initial run
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', addExpandButtons);
    } else {
        addExpandButtons();
    }

    // Re-run on mdBook SPA navigation (content div is replaced on page change)
    var observer = new MutationObserver(function (mutations) {
        for (var i = 0; i < mutations.length; i++) {
            if (mutations[i].addedNodes.length) {
                addExpandButtons();
                break;
            }
        }
    });
    observer.observe(document.body, { childList: true, subtree: true });
}());
