pub const HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>VardaDB Dashboard</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: #111; color: #eee; margin: 0; padding: 20px; }
        .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 20px; }
        .card { background: #222; border-radius: 8px; padding: 15px; border: 1px solid #333; }
        h2 { margin-top: 0; color: #4db8ff; font-size: 1.2rem; }
        .chart-container { height: 200px; position: relative; width: 100%; }
        
        table { width: 100%; border-collapse: collapse; margin-top: 10px; font-size: 0.9rem; }
        th, td { text-align: left; padding: 8px; border-bottom: 1px solid #333; }
        th { color: #888; }
        tr:hover { background: #2a2a2a; }
        
        /* Simple SVG Chart Styles */
        svg { width: 100%; height: 100%; overflow: visible; }
        polyline { fill: none; stroke-width: 2; vector-effect: non-scaling-stroke; }
        .axis { stroke: #444; stroke-width: 1; }
        .tick { fill: #888; font-size: 10px; }
        
        .stat-big { font-size: 2rem; font-weight: bold; margin: 10px 0; }
        .stat-label { color: #888; font-size: 0.8rem; }
    </style>
</head>
<body>
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
        <h1>VardaDB <span style="color: #666; font-weight: normal;">Observability</span></h1>
        <button onclick="refresh()" style="background: #333; color: white; border: none; padding: 8px 16px; border-radius: 4px; cursor: pointer;">Refresh</button>
    </div>

    <div class="grid">
        <div class="card">
            <h2>System CPU & Memory</h2>
            <div id="chart-cpu" class="chart-container"></div>
        </div>
        <div class="card">
            <h2>GraphQL Throughput (RPS)</h2>
             <div id="chart-rps" class="chart-container"></div>
        </div>
        <div class="card">
            <h2>Query Latency</h2>
             <div id="chart-latency" class="chart-container"></div>
        </div>
        <div class="card">
            <h2>Storage Size</h2>
            <div class="stat-big" id="val-storage">-</div>
            <div class="stat-label">Total Validated Bytes</div>
        </div>
    </div>

    <div class="card" style="margin-top: 20px;">
         <h2>Recent Traces</h2>
         <table>
             <thead><tr><th>Time</th><th>Operation</th><th>Duration (ms)</th><th>Fields</th></tr></thead>
             <tbody id="trace-table"></tbody>
         </table>
    </div>

    <script>
        // --- Minimal Chart Lib ---
        function drawChart(containerId, data, color) {
            const container = document.getElementById(containerId);
            container.innerHTML = '';
            if (!data || data.length < 2) return;
            
            // Sort by TS
            data.sort((a, b) => a.t - b.t);
            
            // Bounds
            const minT = data[0].t;
            const maxT = data[data.length-1].t;
            const minV = 0; // Always start at 0 for these metrics usually
            let maxV = Math.max(...data.map(d => d.v));
            if (maxV === 0) maxV = 1;
            
            const w = container.clientWidth;
            const h = container.clientHeight;
            
            // Points to SVG Path
            const points = data.map(d => {
                const x = ((d.t - minT) / (maxT - minT)) * w;
                const y = h - ((d.v - minV) / (maxV - minV)) * h;
                return `${x},${y}`;
            }).join(' ');
            
            const svg = `
                <svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none">
                    <line x1="0" y1="${h}" x2="${w}" y2="${h}" class="axis"/>
                    <line x1="0" y1="0" x2="0" y2="${h}" class="axis"/>
                    <polyline points="${points}" stroke="${color}" />
                </svg>
            `;
            container.innerHTML = svg;
        }

        async function fetchMetrics() {
             const end = Math.floor(Date.now() / 1000);
             const start = end - 3600; // Last hour
             
             try {
                 const res = await fetch(`metrics?start=${start}&end=${end}`);
                 const json = await res.json();
                 
                 // Render CPU
                 if (json['system.cpu']) drawChart('chart-cpu', json['system.cpu'], '#ff4d4d');
                 
                 // RPS (Derived from counter 'graphql_requests_total')
                 // Counter is strictly increasing. We need rate.
                 // Ideally backend sends rate, but here we can compute delta if we had more points.
                 // For now just plot the counter or if we had a gauge rate. 
                 // Let's implement rate calculation in UI? Requires prev fetch.
                 // Simplified: Just plot CPU/Requests for now.
                 if (json['graphql_requests_total']) drawChart('chart-rps', json['graphql_requests_total'], '#4dff88');

             } catch (e) { console.error(e); }
        }

        async function fetchTraces() {
             try {
                 const res = await fetch('traces');
                 const traces = await res.json();
                 const tbody = document.getElementById('trace-table');
                 tbody.innerHTML = traces.map(t => `
                     <tr>
                         <td>${new Date(t.start_ts).toLocaleTimeString()}</td>
                         <td style="color: #fff; font-weight: 500;">${t.name}</td>
                         <td style="${t.duration_ms > 100 ? 'color: #ff4d4d' : 'color: #4dff88'}">${t.duration_ms}ms</td>
                         <td style="color: #888; font-size: 0.8rem;">${JSON.stringify(t.fields).substring(0, 50)}...</td>
                     </tr>
                 `).join('');
             } catch (e) { console.error(e); }
        }

        function refresh() {
            fetchMetrics();
            fetchTraces();
        }
        
        refresh();
        setInterval(refresh, 5000);
    </script>
</body>
</html>
"#;
