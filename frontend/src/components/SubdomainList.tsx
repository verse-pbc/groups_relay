import { Component } from "preact";

interface SubdomainListProps {
  currentSubdomain?: string;
  onSubdomainSelect: (subdomain: string) => void;
  isLoading?: boolean;
}

interface SubdomainListState {
  subdomains: string[];
  isLoadingSubdomains: boolean;
  error: string | null;
}

export class SubdomainList extends Component<SubdomainListProps, SubdomainListState> {
  constructor(props: SubdomainListProps) {
    super(props);
    this.state = {
      subdomains: [],
      isLoadingSubdomains: true,
      error: null,
    };
  }

  async componentDidMount() {
    await this.fetchSubdomains();
  }

  fetchSubdomains = async () => {
    try {
      this.setState({ isLoadingSubdomains: true, error: null });
      
      const response = await fetch('/api/subdomains');
      if (!response.ok) {
        throw new Error(`Failed to fetch subdomains: ${response.status}`);
      }
      
      const data = await response.json();
      this.setState({
        subdomains: data.subdomains || [],
        isLoadingSubdomains: false,
      });
    } catch (error) {
      console.error('Error fetching subdomains:', error);
      this.setState({
        error: error instanceof Error ? error.message : 'Failed to fetch subdomains',
        isLoadingSubdomains: false,
      });
    }
  };

  handleSubdomainClick = (subdomain: string) => {
    this.props.onSubdomainSelect(subdomain);
  };

  handleMainDomainClick = () => {
    this.props.onSubdomainSelect('');
  };

  getCurrentHostWithoutSubdomain = () => {
    const { protocol, hostname, port } = window.location;
    const parts = hostname.split('.');
    
    // For localhost or IP addresses, return as-is
    if (hostname === 'localhost' || hostname.match(/^\d+\.\d+\.\d+\.\d+$/)) {
      return `${protocol}//${hostname}${port ? `:${port}` : ''}`;
    }
    
    // Assume the base domain is the last 2 parts (e.g., example.com)
    // This might need adjustment based on your domain structure
    const baseDomain = parts.slice(-2).join('.');
    return `${protocol}//${baseDomain}${port ? `:${port}` : ''}`;
  };

  navigateToSubdomain = (subdomain: string) => {
    const { protocol, hostname, port } = window.location;
    
    // For localhost or IP addresses, don't try to create subdomains
    if (hostname === 'localhost' || hostname.match(/^\d+\.\d+\.\d+\.\d+$/)) {
      console.warn('Cannot navigate to subdomains on localhost or IP addresses');
      return;
    }
    
    const parts = hostname.split('.');
    const baseDomain = parts.slice(-2).join('.');
    
    const newHostname = subdomain ? `${subdomain}.${baseDomain}` : baseDomain;
    const newUrl = `${protocol}//${newHostname}${port ? `:${port}` : ''}`;
    
    window.location.href = newUrl;
  };

  render() {
    const { currentSubdomain, isLoading } = this.props;
    const { subdomains, isLoadingSubdomains, error } = this.state;

    return (
      <div class="mb-8">
        {/* Communities Header - more prominent */}
        <h3 class="text-base font-semibold text-[var(--color-text-primary)] uppercase tracking-wider mb-4 px-3">Communities</h3>
        
        {error && (
          <div class="text-red-400 text-sm mb-4 px-3 py-2">
            {error}
          </div>
        )}
        
        {isLoadingSubdomains ? (
          <div class="text-[var(--color-text-secondary)] text-sm py-6 text-center opacity-60">
            Loading communities...
          </div>
        ) : (
          <div class="space-y-1 px-3">
            {/* Main community option - minimal design */}
            <button
              onClick={() => this.navigateToSubdomain('')}
              class={`
                w-full px-3 py-3 rounded-lg text-left transition-all duration-200
                ${!currentSubdomain 
                  ? 'bg-white/8 text-[var(--color-text-primary)]' 
                  : 'text-[var(--color-text-secondary)] hover:bg-white/4 hover:text-[var(--color-text-primary)]'
                }
                ${isLoading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
              `}
              disabled={isLoading}
            >
              <div class="flex items-center gap-3">
                <div class="shrink-0 w-6 h-6 flex items-center justify-center text-sm">
                  🌳
                </div>
                <div class="min-w-0">
                  <span class="font-medium truncate">Root</span>
                </div>
              </div>
            </button>
            
            {/* Subdomain options - cleaner cards */}
            {subdomains.map((subdomain) => (
              <button
                key={subdomain}
                onClick={() => this.navigateToSubdomain(subdomain)}
                class={`
                  w-full px-3 py-3 rounded-lg text-left transition-all duration-200
                  ${currentSubdomain === subdomain 
                    ? 'bg-white/8 text-[var(--color-text-primary)]' 
                    : 'text-[var(--color-text-secondary)] hover:bg-white/4 hover:text-[var(--color-text-primary)]'
                  }
                  ${isLoading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                `}
                disabled={isLoading}
              >
                <div class="flex items-center gap-3">
                  <div class="shrink-0 w-6 h-6 bg-white/10 rounded flex items-center justify-center text-xs font-medium">
                    {subdomain.charAt(0).toUpperCase()}
                  </div>
                  <div class="min-w-0">
                    <span class="font-medium truncate">{subdomain}</span>
                  </div>
                </div>
              </button>
            ))}
            
            {subdomains.length === 0 && !isLoadingSubdomains && !error && (
              <div class="text-[var(--color-text-secondary)] text-sm py-6 text-center opacity-40">
                No other communities
              </div>
            )}
          </div>
        )}
        
        {/* Subtle refresh option */}
        {!isLoadingSubdomains && (
          <button
            onClick={this.fetchSubdomains}
            class="w-full text-xs text-[var(--color-text-secondary)]/60 hover:text-[var(--color-text-secondary)] transition-colors py-3 mt-2"
            disabled={isLoadingSubdomains}
          >
            Refresh
          </button>
        )}
      </div>
    );
  }
}