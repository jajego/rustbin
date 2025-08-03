<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { get } from 'svelte/store';

  let socket: WebSocket | null = null;
  let messages: string[] = [];

  // Get the bin ID from the URL
  const binId = get(page).params.id;

  onMount(() => {
    const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${protocol}://localhost:3000/bin/${binId}/ws`;

    socket = new WebSocket(url);

    socket.onmessage = (event) => {
      messages = [...messages, event.data];
    };

    socket.onerror = (e) => {
      console.error('WebSocket error:', e);
    };

    socket.onclose = () => {
      console.log('WebSocket closed');
    };

    return () => {
      socket?.close();
    };
  });
</script>

<h1>Listening to bin: {binId}</h1>

{#if messages.length === 0}
  <p>No messages yet...</p>
{:else}
  <ul>
    {#each messages as msg (msg)}
      <li>{msg}</li>
    {/each}
  </ul>
{/if}
