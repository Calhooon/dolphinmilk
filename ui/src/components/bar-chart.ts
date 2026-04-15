import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import { formatInlineCurrency } from '../lib/usd.js';
import {
  Chart, BarController, BarElement, CategoryScale, LinearScale, Tooltip, Legend,
  DoughnutController, ArcElement,
  LineController, LineElement, PointElement, Filler,
} from 'chart.js';

// Register all needed components
Chart.register(
  BarController, BarElement, CategoryScale, LinearScale, Tooltip, Legend,
  DoughnutController, ArcElement,
  LineController, LineElement, PointElement, Filler,
);

/** Default palette for auto-coloring chart segments. */
const PALETTE = [
  '#14A8C4', '#fcbe2d', '#00b69b', '#a78bfa',
  '#f472b6', '#fb923c', '#38bdf8', '#34d399',
  '#e879f9', '#fbbf24', '#60a5fa', '#f87171',
];

export interface BarData {
  label: string;
  value: number;
  color?: string;
}

/** Data for stacked bar charts — multiple series sharing the same x-axis labels. */
export interface StackedBarData {
  labels: string[];
  series: { label: string; values: number[]; color?: string }[];
}

@customElement('dm-bar-chart')
export class WormBarChart extends LitElement {
  @property({ type: Array }) data: BarData[] = [];
  @property({ attribute: false }) stackedData: StackedBarData | null = null;
  @property({ type: Number }) maxBars = 14;
  @property() height = '200px';
  @property() orientation: 'vertical' | 'horizontal' = 'vertical';
  @property() chartType: 'bar' | 'doughnut' | 'line' | 'stacked-bar' = 'bar';
  @property() centerLabel = '';
  @property({ type: Number }) usdRate = 0;
  @property() currencyMode: CurrencyDisplay = 'usd-first';

  private chart: Chart | null = null;
  private canvas: HTMLCanvasElement | null = null;

  /** Number of data points beyond which the chart becomes horizontally scrollable. */
  private static SCROLL_THRESHOLD = 14;

  /** Whether the chart has enough data points to warrant horizontal scrolling. */
  private get _isScrollable(): boolean {
    if (this.chartType === 'doughnut') return false;
    const count = this.chartType === 'stacked-bar'
      ? (this.stackedData?.labels.length ?? 0)
      : Math.min(this.data.length, this.maxBars);
    return count > WormBarChart.SCROLL_THRESHOLD;
  }

  /** Minimum width for the chart container when scrollable (~45-50px per data point). */
  private get _scrollMinWidth(): string {
    if (!this._isScrollable) return '';
    const count = this.chartType === 'stacked-bar'
      ? (this.stackedData?.labels.length ?? 0)
      : Math.min(this.data.length, this.maxBars);
    const pxPerPoint = this.chartType === 'line' ? 45 : 50;
    return `${count * pxPerPoint}px`;
  }

  static styles = css`
    :host { display: block; }
    .chart-scroll {
      overflow-x: auto;
      -webkit-overflow-scrolling: touch;
    }
    .chart-scroll::-webkit-scrollbar { height: 4px; }
    .chart-scroll::-webkit-scrollbar-thumb { background: var(--border, #1A3550); border-radius: 2px; }
    .chart-scroll::-webkit-scrollbar-track { background: transparent; }
    .chart-container {
      position: relative;
      width: 100%;
    }
    canvas { width: 100% !important; }
    .empty {
      text-align: center;
      padding: 24px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 12px;
    }
  `;

  render() {
    const hasStacked = this.stackedData && this.stackedData.labels.length > 0 && this.stackedData.series.length > 0;
    const visible = this.data.slice(0, this.maxBars);
    if (!hasStacked && visible.length === 0) {
      return html`<div class="empty">No data</div>`;
    }
    const minW = this._scrollMinWidth;
    return html`
      <div class="chart-scroll">
        <div class="chart-container" style="height: ${this.height}${minW ? `; min-width: ${minW}` : ''}">
          <canvas></canvas>
        </div>
      </div>
    `;
  }

  updated(changedProps: Map<string, unknown>) {
    super.updated(changedProps);
    const hasStacked = this.stackedData && this.stackedData.labels.length > 0 && this.stackedData.series.length > 0;
    const visible = this.data.slice(0, this.maxBars);
    if (!hasStacked && visible.length === 0) {
      this.destroyChart();
      return;
    }
    requestAnimationFrame(() => {
      if (this.chartType === 'stacked-bar' && hasStacked) {
        this.destroyChart();
        this.canvas = this.renderRoot.querySelector('canvas');
        if (this.canvas) this.createStackedBarChart(this.stackedData!);
      } else {
        this.createChart(visible);
      }
    });
  }

  private createChart(data: BarData[]) {
    this.destroyChart();
    this.canvas = this.renderRoot.querySelector('canvas');
    if (!this.canvas) return;

    if (this.chartType === 'doughnut') {
      this.createDoughnutChart(data);
    } else if (this.chartType === 'line') {
      this.createLineChart(data);
    } else {
      this.createBarChart(data);
    }
  }

  private createBarChart(data: BarData[]) {
    if (!this.canvas) return;
    const labels = data.map(d => d.label);
    const values = data.map(d => d.value);
    const colors = data.map((d, i) => d.color || PALETTE[i % PALETTE.length]);
    const hoverColors = colors.map(c => c + 'dd');
    const isHorizontal = this.orientation === 'horizontal';
    const monoFont = "'Nunito Sans', 'SF Mono', 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', sans-serif";

    this.chart = new Chart(this.canvas, {
      type: 'bar',
      data: {
        labels,
        datasets: [{
          data: values,
          backgroundColor: colors,
          hoverBackgroundColor: hoverColors,
          borderColor: 'transparent',
          borderWidth: 0,
          borderRadius: 4,
          borderSkipped: false,
          barPercentage: 0.7,
          categoryPercentage: 0.8,
        }],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        indexAxis: isHorizontal ? 'y' : 'x',
        animation: { duration: 400, easing: 'easeOutCubic' },
        layout: { padding: { top: 4, right: 4, bottom: 0, left: 0 } },
        plugins: {
          legend: { display: false },
          tooltip: {
            enabled: true,
            backgroundColor: 'rgba(27, 36, 49, 0.95)',
            titleColor: '#fff',
            bodyColor: '#e0e0e8',
            borderColor: '#14A8C4',
            borderWidth: 1,
            cornerRadius: 8,
            padding: { top: 10, bottom: 10, left: 14, right: 14 },
            titleFont: { family: monoFont, size: 12, weight: 'bold' as const },
            bodyFont: { family: monoFont, size: 12 },
            displayColors: true,
            boxWidth: 10,
            boxHeight: 10,
            boxPadding: 4,
            callbacks: {
              label: (ctx) => {
                const val = ctx.parsed?.[isHorizontal ? 'x' : 'y'] ?? 0;
                if (typeof val !== 'number' || isNaN(val)) return ` ${formatInlineCurrency(0, this.usdRate, this.currencyMode)}`;
                return ` ${formatInlineCurrency(val, this.usdRate, this.currencyMode)}`;
              },
            },
          },
        },
        scales: {
          x: {
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: {
              color: 'rgba(255,255,255,0.5)',
              font: { family: monoFont, size: 10 },
              maxRotation: isHorizontal ? 0 : 45,
              autoSkip: true,
              ...(!isHorizontal && !this._isScrollable ? { maxTicksLimit: 14 } : {}),
              padding: 4,
              ...(isHorizontal ? {
                callback: (val: string | number) => {
                  const n = typeof val === 'number' ? val : Number(val);
                  return n.toLocaleString();
                },
              } : {}),
            },
          },
          y: {
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: {
              color: 'rgba(255,255,255,0.5)',
              font: { family: monoFont, size: 10 },
              padding: 4,
              ...(!isHorizontal ? {
                callback: (val: string | number) => {
                  const n = typeof val === 'number' ? val : Number(val);
                  return n.toLocaleString();
                },
              } : {}),
            },
            beginAtZero: true,
          },
        },
      },
    });
  }

  private createDoughnutChart(data: BarData[]) {
    if (!this.canvas) return;
    const monoFont = "'Nunito Sans', 'SF Mono', 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', sans-serif";
    const colors = data.map((d, i) => d.color || PALETTE[i % PALETTE.length]);

    this.chart = new Chart(this.canvas, {
      type: 'doughnut',
      data: {
        labels: data.map(d => d.label),
        datasets: [{
          data: data.map(d => d.value),
          backgroundColor: colors,
          hoverBackgroundColor: colors.map(c => c + 'cc'),
          borderColor: '#0B1929',
          borderWidth: 2,
          hoverBorderColor: '#fff',
          hoverBorderWidth: 2,
          hoverOffset: 6,
        }],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        cutout: '65%',
        animation: { duration: 600, easing: 'easeOutCubic' },
        plugins: {
          legend: {
            display: true,
            position: 'bottom',
            labels: {
              color: 'rgba(255,255,255,0.6)',
              font: { family: monoFont, size: 11 },
              padding: 14,
              usePointStyle: true,
              pointStyleWidth: 8,
            },
          },
          tooltip: {
            enabled: true,
            backgroundColor: 'rgba(27, 36, 49, 0.95)',
            titleColor: '#fff',
            bodyColor: '#e0e0e8',
            borderColor: '#14A8C4',
            borderWidth: 1,
            cornerRadius: 8,
            padding: { top: 10, bottom: 10, left: 14, right: 14 },
            titleFont: { family: monoFont, size: 12, weight: 'bold' as const },
            bodyFont: { family: monoFont, size: 12 },
            displayColors: true,
            boxWidth: 10,
            boxHeight: 10,
            boxPadding: 4,
            callbacks: {
              label: (ctx) => {
                const val = ctx.parsed ?? 0;
                if (typeof val !== 'number' || isNaN(val)) return ` ${ctx.label ?? ''}: ${formatInlineCurrency(0, this.usdRate, this.currencyMode)}`;
                const dataArr = (ctx.dataset.data as number[]) ?? [];
                const total = dataArr.reduce((s, v) => s + (v ?? 0), 0);
                const pct = total > 0 ? ((val / total) * 100).toFixed(1) : '0';
                return ` ${ctx.label ?? ''}: ${formatInlineCurrency(val, this.usdRate, this.currencyMode)} — ${pct}%`;
              },
            },
          },
        },
      },
    });
  }

  private createLineChart(data: BarData[]) {
    if (!this.canvas) return;
    const monoFont = "'Nunito Sans', 'SF Mono', 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', sans-serif";
    const ctx = this.canvas.getContext('2d');
    let gradient: CanvasGradient | string = 'rgba(20, 168, 196, 0.15)';
    if (ctx) {
      gradient = ctx.createLinearGradient(0, 0, 0, this.canvas.height || 200);
      gradient.addColorStop(0, 'rgba(20, 168, 196, 0.25)');
      gradient.addColorStop(0.6, 'rgba(20, 168, 196, 0.08)');
      gradient.addColorStop(1, 'rgba(20, 168, 196, 0.01)');
    }

    this.chart = new Chart(this.canvas, {
      type: 'line',
      data: {
        labels: data.map(d => d.label),
        datasets: [{
          data: data.map(d => d.value),
          borderColor: '#14A8C4',
          backgroundColor: gradient,
          borderWidth: 2,
          fill: true,
          tension: 0.35,
          pointRadius: 0,
          pointHoverRadius: 6,
          pointHoverBackgroundColor: '#14A8C4',
          pointHoverBorderColor: '#fff',
          pointHoverBorderWidth: 2,
        }],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        interaction: {
          mode: 'index',
          intersect: false,
        },
        animation: { duration: 400, easing: 'easeOutCubic' },
        layout: { padding: { top: 4, right: 4, bottom: 0, left: 0 } },
        plugins: {
          legend: { display: false },
          tooltip: {
            enabled: true,
            backgroundColor: 'rgba(27, 36, 49, 0.95)',
            titleColor: '#fff',
            bodyColor: '#e0e0e8',
            borderColor: '#14A8C4',
            borderWidth: 1,
            cornerRadius: 8,
            padding: { top: 10, bottom: 10, left: 14, right: 14 },
            titleFont: { family: monoFont, size: 12, weight: 'bold' as const },
            bodyFont: { family: monoFont, size: 12 },
            displayColors: false,
            callbacks: {
              label: (ctx) => {
                const val = ctx.parsed?.y ?? 0;
                if (typeof val !== 'number' || isNaN(val)) return formatInlineCurrency(0, this.usdRate, this.currencyMode);
                return formatInlineCurrency(val, this.usdRate, this.currencyMode);
              },
            },
          },
        },
        scales: {
          x: {
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: { color: 'rgba(255,255,255,0.5)', font: { family: monoFont, size: 10 }, padding: 4, maxRotation: 45, autoSkip: true, ...(this._isScrollable ? {} : { maxTicksLimit: 12 }) },
          },
          y: {
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: {
              color: 'rgba(255,255,255,0.5)',
              font: { family: monoFont, size: 10 },
              padding: 4,
              callback: (val: string | number) => {
                const n = typeof val === 'number' ? val : Number(val);
                return n.toLocaleString();
              },
            },
            beginAtZero: true,
          },
        },
      },
    });
  }

  private createStackedBarChart(data: StackedBarData) {
    if (!this.canvas) return;
    const monoFont = "'Nunito Sans', 'SF Mono', 'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', sans-serif";

    const datasets = data.series.map((s, i) => ({
      label: s.label,
      data: s.values,
      backgroundColor: s.color || PALETTE[i % PALETTE.length],
      hoverBackgroundColor: (s.color || PALETTE[i % PALETTE.length]) + 'dd',
      borderColor: 'transparent',
      borderWidth: 0,
      borderRadius: 3,
      borderSkipped: false as const,
      barPercentage: 0.7,
      categoryPercentage: 0.85,
    }));

    this.chart = new Chart(this.canvas, {
      type: 'bar',
      data: { labels: data.labels, datasets },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        animation: { duration: 400, easing: 'easeOutCubic' },
        layout: { padding: { top: 4, right: 4, bottom: 0, left: 0 } },
        plugins: {
          legend: {
            display: true,
            position: 'bottom',
            labels: {
              color: 'rgba(255,255,255,0.6)',
              font: { family: monoFont, size: 11 },
              padding: 14,
              usePointStyle: true,
              pointStyleWidth: 8,
            },
          },
          tooltip: {
            enabled: true,
            mode: 'index',
            backgroundColor: 'rgba(27, 36, 49, 0.95)',
            titleColor: '#fff',
            bodyColor: '#e0e0e8',
            borderColor: '#14A8C4',
            borderWidth: 1,
            cornerRadius: 8,
            padding: { top: 10, bottom: 10, left: 14, right: 14 },
            titleFont: { family: monoFont, size: 12, weight: 'bold' as const },
            bodyFont: { family: monoFont, size: 12 },
            displayColors: true,
            boxWidth: 10,
            boxHeight: 10,
            boxPadding: 4,
            filter: (item) => (item.parsed?.y ?? 0) > 0,
            callbacks: {
              label: (ctx) => {
                const val = ctx.parsed?.y ?? 0;
                if (typeof val !== 'number' || isNaN(val) || val === 0) return '';
                return ` ${ctx.dataset.label}: ${formatInlineCurrency(val, this.usdRate, this.currencyMode)}`;
              },
              footer: (items) => {
                const total = items.reduce((s, i) => s + (i.parsed?.y ?? 0), 0);
                if (total <= 0) return '';
                return `Total: ${formatInlineCurrency(total, this.usdRate, this.currencyMode)}`;
              },
            },
          },
        },
        scales: {
          x: {
            stacked: true,
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: { color: 'rgba(255,255,255,0.5)', font: { family: monoFont, size: 10 }, padding: 4, autoSkip: true, ...(this._isScrollable ? {} : { maxTicksLimit: 14 }) },
          },
          y: {
            stacked: true,
            grid: { color: 'rgba(49, 61, 79, 0.5)', drawTicks: false },
            border: { color: 'rgba(49, 61, 79, 0.5)' },
            ticks: {
              color: 'rgba(255,255,255,0.5)',
              font: { family: monoFont, size: 10 },
              padding: 4,
              callback: (val: string | number) => {
                const n = typeof val === 'number' ? val : Number(val);
                return n.toLocaleString();
              },
            },
            beginAtZero: true,
          },
        },
      },
    });
  }

  private destroyChart() {
    if (this.chart) {
      this.chart.destroy();
      this.chart = null;
    }
    this.canvas = null;
  }

  disconnectedCallback() {
    this.destroyChart();
    super.disconnectedCallback();
  }
}
