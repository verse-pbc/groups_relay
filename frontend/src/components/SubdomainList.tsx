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
      <div class="bg-[var(--color-bg-secondary)] border border-[var(--color-border)] rounded-lg p-4 mb-4">
        <h3 class="text-lg font-semibold mb-3 text-[var(--color-text-primary)]">
          Domains
        </h3>
        
        {error && (
          <div class="text-red-500 text-sm mb-3 p-2 bg-red-50 rounded">
            {error}
          </div>
        )}
        
        {isLoadingSubdomains ? (
          <div class="text-[var(--color-text-secondary)] text-sm animate-pulse">
            Loading domains...
          </div>
        ) : (
          <div class="space-y-2">
            {/* Main domain option */}
            <button
              onClick={() => this.navigateToSubdomain('')}
              class={`
                w-full text-left px-3 py-2 rounded-md transition-colors text-sm
                ${!currentSubdomain 
                  ? 'bg-[var(--color-accent)] text-white' 
                  : 'hover:bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]'
                }
                ${isLoading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
              `}
              disabled={isLoading}
            >
              <span class="font-medium">Main</span>
              <span class="text-xs opacity-75 block">Primary domain</span>
            </button>
            
            {/* Subdomain options */}
            {subdomains.map((subdomain) => (
              <button
                key={subdomain}
                onClick={() => this.navigateToSubdomain(subdomain)}
                class={`
                  w-full text-left px-3 py-2 rounded-md transition-colors text-sm
                  ${currentSubdomain === subdomain 
                    ? 'bg-[var(--color-accent)] text-white' 
                    : 'hover:bg-[var(--color-bg-primary)] text-[var(--color-text-primary)]'
                  }
                  ${isLoading ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                `}
                disabled={isLoading}
              >
                <span class="font-medium">{subdomain}</span>
                <span class="text-xs opacity-75 block">{subdomain}.domain.com</span>
              </button>
            ))}
            
            {subdomains.length === 0 && !isLoadingSubdomains && !error && (
              <div class="text-[var(--color-text-secondary)] text-sm italic">
                No subdomains available
              </div>
            )}
          </div>
        )}
        
        {/* Refresh button */}
        <button
          onClick={this.fetchSubdomains}
          class="mt-3 w-full text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
          disabled={isLoadingSubdomains}
        >
          {isLoadingSubdomains ? 'Refreshing...' : 'Refresh domains'}
        </button>
      </div>
    );
  }
}