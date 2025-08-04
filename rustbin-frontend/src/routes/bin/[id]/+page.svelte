<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { get } from 'svelte/store';

  interface ParsedRequest {
    id: string;
    method: string;
    path: string;
    headers: Record<string, string>;
    body: string;
    timestamp: Date;
    ip?: string;
    userAgent?: string;
  }

  let socket: WebSocket | null = null;
  let requests: ParsedRequest[] = [];
  let isConnected = false;
  let isLoading = true;
  let loadError: string | null = null;

  // Get the bin ID from the URL
  const binId = get(page).params.id;

  interface ApiLoggedRequest {
    method: string;
    headers: string;
    body: string | null;
    timestamp: string;
    request_id: string;
  }

  function convertApiRequestToParsedRequest(apiRequest: ApiLoggedRequest): ParsedRequest {
    // Parse headers string to object
    let headers: Record<string, string> = {};
    try {
      if (apiRequest.headers) {
        headers = JSON.parse(apiRequest.headers);
      }
    } catch {
      // If parsing fails, treat as raw text
      headers = { 'raw': apiRequest.headers };
    }

    return {
      id: apiRequest.request_id,
      method: apiRequest.method,
      path: headers['path'] || headers['url'] || '/', // Extract path from headers if available
      headers,
      body: apiRequest.body || '',
      timestamp: new Date(apiRequest.timestamp),
      ip: headers['x-forwarded-for'] || headers['x-real-ip'],
      userAgent: headers['user-agent']
    };
  }

  async function fetchExistingRequests() {
    try {
      isLoading = true;
      loadError = null;
      
      const response = await fetch(`https://api.rustb.in/bin/${binId}/inspect`);
      
      if (response.status === 404) {
        loadError = 'bin-not-found';
        return;
      }
      
      if (!response.ok) {
        throw new Error(`Failed to fetch requests: ${response.status} ${response.statusText}`);
      }
      
      const apiRequests: ApiLoggedRequest[] = await response.json();
      
      // Convert API format to frontend format and sort by timestamp (newest first)
      const convertedRequests = apiRequests
        .map(convertApiRequestToParsedRequest)
        .sort((a, b) => b.timestamp.getTime() - a.timestamp.getTime());
      
      requests = convertedRequests;
    } catch (error) {
      console.error('Failed to fetch existing requests:', error);
      loadError = error instanceof Error ? error.message : 'Failed to load requests';
    } finally {
      isLoading = false;
    }
  }

  function parseHttpRequest(raw: string): ParsedRequest {
    try {
      // Try to parse as JSON first (if the backend sends structured data)
      const data = JSON.parse(raw);
      
      // Handle headers properly - they might come as string, array, or object
      let headers: Record<string, string> = {};
      if (data.headers) {
        if (typeof data.headers === 'string') {
          // If headers come as a string, try to parse them
          try {
            headers = JSON.parse(data.headers);
          } catch {
            // If parsing fails, treat as raw text
            headers = { 'raw': data.headers };
          }
        } else if (Array.isArray(data.headers)) {
          // If headers come as an array, convert to object
          headers = {};
          for (let i = 0; i < data.headers.length; i += 2) {
            if (data.headers[i] && data.headers[i + 1]) {
              headers[data.headers[i]] = data.headers[i + 1];
            }
          }
        } else if (typeof data.headers === 'object') {
          headers = data.headers;
        }
      }
      
      return {
        id: crypto.randomUUID(),
        method: data.method || 'UNKNOWN',
        path: data.path || data.url || '/',
        headers,
        body: data.body || '',
        timestamp: new Date(),
        ip: data.ip || data.remote_addr,
        userAgent: headers['user-agent'] || headers['User-Agent']
      };
    } catch {
      // Fallback: parse raw HTTP request string
      const lines = raw.split('\n');
      const requestLine = lines[0] || '';
      const [method, path] = requestLine.split(' ');
      
      const headers: Record<string, string> = {};
      let bodyStart = -1;
      
      for (let i = 1; i < lines.length; i++) {
        if (lines[i].trim() === '') {
          bodyStart = i + 1;
          break;
        }
        const colonIndex = lines[i].indexOf(':');
        if (colonIndex > 0) {
          const key = lines[i].substring(0, colonIndex).trim();
          const value = lines[i].substring(colonIndex + 1).trim();
          if (key && value) headers[key] = value;
        }
      }
      
      const body = bodyStart > -1 ? lines.slice(bodyStart).join('\n') : '';
      
      return {
        id: crypto.randomUUID(),
        method: method || 'UNKNOWN',
        path: path || '/',
        headers,
        body,
        timestamp: new Date(),
        ip: headers['X-Forwarded-For'] || headers['X-Real-IP'],
        userAgent: headers['User-Agent']
      };
    }
  }

  function getMethodColor(method: string): string {
    const colors: Record<string, string> = {
      'GET': 'bg-green-100 text-green-800 border-green-200',
      'POST': 'bg-blue-100 text-blue-800 border-blue-200',
      'PUT': 'bg-yellow-100 text-yellow-800 border-yellow-200',
      'PATCH': 'bg-orange-100 text-orange-800 border-orange-200',
      'DELETE': 'bg-red-100 text-red-800 border-red-200',
      'OPTIONS': 'bg-purple-100 text-purple-800 border-purple-200',
      'HEAD': 'bg-gray-100 text-gray-800 border-gray-200'
    };
    return colors[method] || 'bg-gray-100 text-gray-800 border-gray-200';
  }

  function formatTime(date: Date): string {
    return date.toLocaleTimeString('en-US', { 
      hour12: false, 
      hour: '2-digit', 
      minute: '2-digit', 
      second: '2-digit' 
    });
  }

  function clearRequests() {
    requests = [];
  }

  onMount(() => {
    let cleanup: (() => void) | undefined;

    const initializeConnection = async () => {
      // First, fetch existing requests
      await fetchExistingRequests();
      
      // Don't establish WebSocket connection if bin doesn't exist
      if (loadError === 'bin-not-found') {
        return;
      }
      
      // Then establish WebSocket connection for real-time updates
      const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
      const url = `${protocol}://api.rustb.in/bin/${binId}/ws`;

      socket = new WebSocket(url);

      socket.onopen = () => {
        isConnected = true;
      };

      socket.onmessage = (event) => {
        const parsed = parseHttpRequest(event.data);
        
        // Check if this request already exists (avoid duplicates)
        const existingIndex = requests.findIndex(req => req.id === parsed.id);
        if (existingIndex === -1) {
          // Add new request to the top of the list
          requests = [parsed, ...requests];
        } else {
          // Update existing request (in case data was incomplete initially)
          requests[existingIndex] = parsed;
          requests = requests;
        }
      };

      socket.onerror = (e) => {
        console.error('WebSocket error:', e);
        isConnected = false;
      };

      socket.onclose = () => {
        console.log('WebSocket closed');
        isConnected = false;
      };

      cleanup = () => {
        socket?.close();
      };
    };

    initializeConnection();

    return () => {
      cleanup?.();
    };
  });
</script>

<!-- Modern Header -->
<div class="min-h-screen bg-gray-50 fallback-container">
  <div class="max-w-7xl mx-auto px-4 py-6 fallback-wrapper">
    <!-- Header Section -->
    <div class="flex items-center justify-between mb-6 fallback-header">
      <div class="flex items-center space-x-3 fallback-title">
        <div class="w-2 h-2 rounded-full fallback-status-dot {isConnected ? 'bg-green-500 animate-pulse connected' : 'bg-red-500'}"></div>
        <h1 class="text-2xl font-bold text-gray-900">
          Request Bin <span class="text-blue-600">#{binId}</span>
        </h1>
        <span class="text-sm text-gray-500">
          {isConnected ? 'Connected' : 'Disconnected'}
        </span>
      </div>
      
      <button 
        on:click={clearRequests}
        class="px-3 py-1.5 bg-red-500 hover:bg-red-600 text-white text-sm rounded-md font-medium transition-colors fallback-button"
      >
        Clear All
      </button>
    </div>

    <!-- Stats Bar -->
    <div class="grid grid-cols-1 md:grid-cols-3 gap-4 mb-6 fallback-stats">
      <div class="bg-white rounded-lg shadow-sm border p-4 fallback-stat-card">
        <div class="flex items-center fallback-stat-header">
          <div class="w-8 h-8 bg-blue-100 rounded-lg flex items-center justify-center fallback-icon">
            <svg class="w-4 h-4 text-blue-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"></path>
            </svg>
          </div>
          <div class="ml-3">
            <p class="text-xs text-gray-600 fallback-stat-label">Total Requests</p>
            <p class="text-lg font-semibold text-gray-900 fallback-stat-value">{requests.length}</p>
          </div>
        </div>
      </div>
      
      <div class="bg-white rounded-lg shadow-sm border p-4 fallback-stat-card">
        <div class="flex items-center fallback-stat-header">
          <div class="w-8 h-8 bg-green-100 rounded-lg flex items-center justify-center fallback-icon">
            <svg class="w-4 h-4 text-green-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
            </svg>
          </div>
          <div class="ml-3">
            <p class="text-xs text-gray-600 fallback-stat-label">Status</p>
            <p class="text-lg font-semibold fallback-stat-value {isConnected ? 'text-green-600' : 'text-red-600'}">
              {isConnected ? 'Live' : 'Down'}
            </p>
          </div>
        </div>
      </div>
      
      <div class="bg-white rounded-lg shadow-sm border p-4 fallback-stat-card">
        <div class="flex items-center fallback-stat-header">
          <div class="w-8 h-8 bg-gray-100 rounded-lg flex items-center justify-center fallback-icon">
            <svg class="w-4 h-4 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"></path>
            </svg>
          </div>
          <div class="ml-3">
            <p class="text-xs text-gray-600 fallback-stat-label">Last Request</p>
            <p class="text-sm font-semibold text-gray-900 fallback-stat-value">
              {requests.length > 0 ? formatTime(requests[0].timestamp) : 'None'}
            </p>
          </div>
        </div>
      </div>
    </div>

    <!-- Request Cards Section -->
    {#if isLoading}
      <div class="text-center py-12 fallback-empty">
        <div class="w-16 h-16 mx-auto mb-4 bg-blue-100 rounded-lg flex items-center justify-center fallback-empty-icon">
          <svg class="w-8 h-8 text-blue-600 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
          </svg>
        </div>
        <h3 class="text-lg font-semibold text-gray-900 mb-2">Loading requests...</h3>
        <p class="text-gray-600 text-sm max-w-md mx-auto">
          Fetching existing requests for this bin...
        </p>
      </div>
    {:else if loadError === 'bin-not-found'}
      <div class="text-center py-12 fallback-empty">
        <div class="w-16 h-16 mx-auto mb-4 bg-yellow-100 rounded-lg flex items-center justify-center fallback-empty-icon">
          <svg class="w-8 h-8 text-yellow-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.172 16.172a4 4 0 015.656 0M9 12h6m-6 4h6m6 5H3a2 2 0 01-2-2V5a2 2 0 012-2h14a2 2 0 012 2v14a2 2 0 01-2 2z"></path>
          </svg>
        </div>
        <h3 class="text-lg font-semibold text-gray-900 mb-2">Bin Not Found</h3>
        <p class="text-gray-600 text-sm max-w-md mx-auto mb-4">
          The bin with ID <code class="bg-gray-100 px-1 py-0.5 rounded text-xs font-mono">{binId}</code> doesn't exist.
        </p>
        <p class="text-gray-500 text-xs max-w-md mx-auto mb-4">
          It may have been deleted or you might have an incorrect URL.
        </p>
        <div class="flex gap-2 justify-center">
          <button 
            on:click={() => window.location.href = '/'}
            class="px-3 py-1.5 bg-blue-500 hover:bg-blue-600 text-white text-sm rounded-md font-medium transition-colors fallback-button"
          >
            Create New Bin
          </button>
          <button 
            on:click={fetchExistingRequests}
            class="px-3 py-1.5 bg-gray-500 hover:bg-gray-600 text-white text-sm rounded-md font-medium transition-colors fallback-button"
          >
            Try Again
          </button>
        </div>
      </div>
    {:else if loadError}
      <div class="text-center py-12 fallback-empty">
        <div class="w-16 h-16 mx-auto mb-4 bg-red-100 rounded-lg flex items-center justify-center fallback-empty-icon">
          <svg class="w-8 h-8 text-red-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.958-.833-2.728 0L4.088 16.5c-.77.833.192 2.5 1.732 2.5z"></path>
          </svg>
        </div>
        <h3 class="text-lg font-semibold text-gray-900 mb-2">Failed to load requests</h3>
        <p class="text-gray-600 text-sm max-w-md mx-auto mb-4">
          {loadError}
        </p>
        <button 
          on:click={fetchExistingRequests}
          class="px-3 py-1.5 bg-blue-500 hover:bg-blue-600 text-white text-sm rounded-md font-medium transition-colors fallback-button"
        >
          Try Again
        </button>
      </div>
    {:else if requests.length === 0}
      <div class="text-center py-12 fallback-empty">
        <div class="w-16 h-16 mx-auto mb-4 bg-gray-100 rounded-lg flex items-center justify-center fallback-empty-icon">
          <svg class="w-8 h-8 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"></path>
          </svg>
        </div>
        <h3 class="text-lg font-semibold text-gray-900 mb-2">Waiting for requests...</h3>
        <p class="text-gray-600 text-sm max-w-md mx-auto">
          Send HTTP requests to your bin endpoint to see them appear here.
        </p>
      </div>
    {:else}
      <div class="space-y-4 fallback-requests">
        {#each requests as request (request.id)}
          <div class="bg-white rounded-lg shadow-sm border hover:shadow-md transition-shadow fallback-request-card">
            <!-- Card Header -->
            <div class="px-4 py-3 border-b bg-gray-50 rounded-t-lg fallback-request-header">
              <div class="flex items-center justify-between">
                <div class="flex items-center space-x-3 fallback-request-info">
                  <span class="px-2 py-1 text-xs font-medium rounded-md fallback-method-badge {request.method.toLowerCase() === 'get' ? 'fallback-method-get' : request.method.toLowerCase() === 'post' ? 'fallback-method-post' : 'fallback-method-default'} {getMethodColor(request.method)}">
                    {request.method}
                  </span>
                  <code class="text-sm font-mono text-gray-700 bg-white px-2 py-1 rounded border fallback-path">
                    {request.path}
                  </code>
                </div>
                <div class="flex items-center space-x-3 text-xs text-gray-500 fallback-request-meta">
                  {#if request.ip}
                    <span class="flex items-center">
                      <svg class="w-3 h-3 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9v-9m0-9v9"></path>
                      </svg>
                      {request.ip}
                    </span>
                  {/if}
                  <span class="flex items-center">
                    <svg class="w-3 h-3 mr-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                    </svg>
                    {formatTime(request.timestamp)}
                  </span>
                </div>
              </div>
            </div>

            <!-- Card Body -->
            <div class="p-4 fallback-request-body">
              <div class="grid md:grid-cols-2 gap-4">
                <!-- Headers Section -->
                <div>
                  <h4 class="text-sm font-medium text-gray-900 mb-2 flex items-center fallback-section-title">
                    <svg class="w-3 h-3 mr-1 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"></path>
                    </svg>
                    Headers
                  </h4>
                  <div class="bg-gray-50 rounded-md p-2 max-h-32 overflow-y-auto text-xs fallback-section">
                    {#if Object.keys(request.headers).length > 0}
                      <div class="space-y-1">
                        {#each Object.entries(request.headers) as [key, value]}
                          <div class="flex fallback-header-item">
                            <span class="font-medium text-gray-700 min-w-0 flex-shrink-0 fallback-header-key">{key}:</span>
                            <span class="text-gray-600 ml-2 break-all fallback-header-value">{value}</span>
                          </div>
                        {/each}
                      </div>
                    {:else}
                      <p class="text-gray-500 italic fallback-empty-text">No headers</p>
                    {/if}
                  </div>
                </div>

                <!-- Body Section -->
                <div>
                  <h4 class="text-sm font-medium text-gray-900 mb-2 flex items-center fallback-section-title">
                    <svg class="w-3 h-3 mr-1 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path>
                    </svg>
                    Body
                  </h4>
                  <div class="bg-gray-50 rounded-md p-2 max-h-32 overflow-y-auto fallback-section">
                    {#if request.body.trim()}
                      <pre class="text-xs text-gray-700 whitespace-pre-wrap break-all font-mono fallback-body-content">{request.body}</pre>
{:else}
                      <p class="text-xs text-gray-500 italic fallback-empty-text">No body content</p>
                    {/if}
                  </div>
                </div>
              </div>

              <!-- User Agent (if available) -->
              {#if request.userAgent}
                <div class="mt-3 pt-3 border-t">
                  <h4 class="text-sm font-medium text-gray-900 mb-1 flex items-center fallback-section-title">
                    <svg class="w-3 h-3 mr-1 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"></path>
                    </svg>
                    User Agent
                  </h4>
                  <p class="text-xs text-gray-600 bg-gray-50 rounded-md p-2 break-all font-mono fallback-body-content">
                    {request.userAgent}
                  </p>
                </div>
              {/if}
            </div>
          </div>
    {/each}
      </div>
{/if}
  </div>
</div>

<style>
  /* Fallback styles in case Tailwind isn't loading */
  .fallback-container {
    min-height: 100vh;
    background-color: #f8fafc;
    font-family: system-ui, -apple-system, sans-serif;
  }
  
  .fallback-wrapper {
    max-width: 1200px;
    margin: 0 auto;
    padding: 1.5rem;
  }
  
  .fallback-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 1.5rem;
  }
  
  .fallback-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: #1f2937;
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  
  .fallback-status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: #ef4444;
  }
  
  .fallback-status-dot.connected {
    background-color: #22c55e;
    animation: pulse 2s infinite;
  }
  
  .fallback-button {
    padding: 0.5rem 1rem;
    background-color: #ef4444;
    color: white;
    border: none;
    border-radius: 0.375rem;
    font-weight: 500;
    cursor: pointer;
    font-size: 0.875rem;
  }
  
  .fallback-button:hover {
    background-color: #dc2626;
  }
  
  .fallback-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 1.5rem;
  }
  
  .fallback-stat-card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 0.5rem;
    padding: 1rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }
  
  .fallback-stat-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  
  .fallback-icon {
    width: 32px;
    height: 32px;
    background-color: #dbeafe;
    border-radius: 0.5rem;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  
  .fallback-icon svg {
    width: 16px;
    height: 16px;
    color: #2563eb;
  }
  
  .fallback-stat-label {
    font-size: 0.75rem;
    color: #6b7280;
    margin: 0;
  }
  
  .fallback-stat-value {
    font-size: 1.125rem;
    font-weight: 600;
    color: #1f2937;
    margin: 0;
  }
  
  .fallback-empty {
    text-align: center;
    padding: 3rem 0;
  }
  
  .fallback-empty-icon {
    width: 64px;
    height: 64px;
    background-color: #f3f4f6;
    border-radius: 0.5rem;
    margin: 0 auto 1rem;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  
  .fallback-empty-icon svg {
    width: 32px;
    height: 32px;
    color: #9ca3af;
  }
  
  .fallback-requests {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  
  .fallback-request-card {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 0.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    overflow: hidden;
  }
  
  .fallback-request-card:hover {
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
  }
  
  .fallback-request-header {
    padding: 0.75rem 1rem;
    background-color: #f9fafb;
    border-bottom: 1px solid #e5e7eb;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  
  .fallback-request-info {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  
  .fallback-method-badge {
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
    font-weight: 500;
    border-radius: 0.375rem;
    border: 1px solid;
  }
  
  .fallback-method-get {
    background-color: #dcfce7;
    color: #166534;
    border-color: #bbf7d0;
  }
  
  .fallback-method-post {
    background-color: #dbeafe;
    color: #1e40af;
    border-color: #bfdbfe;
  }
  
  .fallback-method-default {
    background-color: #f3f4f6;
    color: #374151;
    border-color: #d1d5db;
  }
  
  .fallback-path {
    font-family: ui-monospace, 'Courier New', monospace;
    font-size: 0.875rem;
    background-color: white;
    padding: 0.25rem 0.5rem;
    border: 1px solid #e5e7eb;
    border-radius: 0.25rem;
    color: #374151;
  }
  
  .fallback-request-meta {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    font-size: 0.75rem;
    color: #6b7280;
  }
  
  .fallback-request-body {
    padding: 1rem;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }
  
  .fallback-section {
    background-color: #f9fafb;
    border-radius: 0.375rem;
    padding: 0.75rem;
    max-height: 128px;
    overflow-y: auto;
  }
  
  .fallback-section-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: #1f2937;
    margin: 0 0 0.5rem 0;
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  
  .fallback-section-title svg {
    width: 12px;
    height: 12px;
    color: #6b7280;
  }
  
  .fallback-header-item {
    display: flex;
    margin-bottom: 0.25rem;
    font-size: 0.75rem;
  }
  
  .fallback-header-key {
    font-weight: 500;
    color: #374151;
    flex-shrink: 0;
  }
  
  .fallback-header-value {
    color: #6b7280;
    margin-left: 0.5rem;
    word-break: break-all;
  }
  
  .fallback-body-content {
    font-family: ui-monospace, 'Courier New', monospace;
    font-size: 0.75rem;
    color: #374151;
    white-space: pre-wrap;
    word-break: break-all;
  }
  
  .fallback-empty-text {
    font-size: 0.75rem;
    color: #9ca3af;
    font-style: italic;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .animate-spin {
    animation: spin 1s linear infinite;
  }
  
  /* Mobile responsive */
  @media (max-width: 768px) {
    .fallback-header {
      flex-direction: column;
      gap: 1rem;
      align-items: stretch;
    }
    
    .fallback-request-body {
      grid-template-columns: 1fr;
    }
    
    .fallback-request-header {
      flex-direction: column;
      gap: 0.5rem;
      align-items: stretch;
    }
    
    .fallback-request-meta {
      justify-content: space-between;
    }
  }
</style>
