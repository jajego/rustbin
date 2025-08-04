<script lang="ts">
  import { goto } from '$app/navigation';

  let isCreating = false;

  async function createBin() {
    if (isCreating) return;
    
    try {
      isCreating = true;
      
      const response = await fetch('https://api.rustb.in/create', {
        method: 'POST'
      });
      
      if (response.ok) {
        const result = await response.json();
        const binId = result.id || result.bin_id; // Handle different response formats
        
        if (binId) {
          goto(`/bin/${binId}`);
        } else {
          console.error('No bin ID in response:', result);
        }
      } else {
        console.error('Failed to create bin:', response.status, response.statusText);
      }
    } catch (error) {
      console.error('Failed to create bin:', error);
    } finally {
      isCreating = false;
    }
  }
</script>

<div class="min-h-screen bg-gray-50 flex items-center justify-center p-4">
  <div class="max-w-md w-full">
    <!-- Header -->
    <div class="text-center mb-8">
      <h1 class="text-4xl font-bold text-gray-900 mb-4">rustbin</h1>
      <p class="text-lg text-gray-600 mb-2">
        Debug webhooks and HTTP requests
      </p>
      <p class="text-sm text-gray-500">
        Create a temporary endpoint to capture and inspect HTTP requests in real-time
      </p>
    </div>

    <!-- Create Bin Button -->
    <div class="bg-white rounded-lg shadow-lg p-6 text-center">
      <button 
        on:click={createBin}
        disabled={isCreating}
        class="w-full px-6 py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white font-semibold rounded-lg transition-colors text-lg"
      >
        {isCreating ? 'Creating...' : 'Create New Bin'}
      </button>
      
      <p class="text-xs text-gray-500 mt-3">
        No signup required • Free to use • Temporary storage
      </p>
    </div>

    <!-- Features -->
    <div class="mt-8 grid grid-cols-1 gap-4">
      <div class="bg-white rounded-lg p-4 shadow-sm">
        <div class="flex items-center">
          <div class="w-8 h-8 bg-green-100 rounded-lg flex items-center justify-center mr-3">
            <svg class="w-4 h-4 text-green-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"></path>
            </svg>
          </div>
          <div>
            <h3 class="font-medium text-gray-900">Real-time Monitoring</h3>
            <p class="text-sm text-gray-600">See requests as they happen with WebSocket updates</p>
          </div>
        </div>
      </div>

      <div class="bg-white rounded-lg p-4 shadow-sm">
        <div class="flex items-center">
          <div class="w-8 h-8 bg-blue-100 rounded-lg flex items-center justify-center mr-3">
            <svg class="w-4 h-4 text-blue-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path>
            </svg>
          </div>
          <div>
            <h3 class="font-medium text-gray-900">Full Request Details</h3>
            <p class="text-sm text-gray-600">Headers, body, method, and metadata captured</p>
          </div>
        </div>
      </div>

      <div class="bg-white rounded-lg p-4 shadow-sm">
        <div class="flex items-center">
          <div class="w-8 h-8 bg-purple-100 rounded-lg flex items-center justify-center mr-3">
            <svg class="w-4 h-4 text-purple-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"></path>
            </svg>
          </div>
          <div>
            <h3 class="font-medium text-gray-900">Self-Hostable</h3>
            <p class="text-sm text-gray-600">Deploy your own instance with simple configuration</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</div>
