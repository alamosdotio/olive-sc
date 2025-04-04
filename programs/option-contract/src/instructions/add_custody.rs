use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::state::{Contract, Custody, Multisig, Pool};

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct AddCustodyParams {
    pub oracle: Pubkey,
    pub pool_name : String
}

pub fn add_custody<'info>(
    ctx: Context<'_, '_, '_, 'info, AddCustody<'info>>,
    params: &AddCustodyParams,
) -> Result<u8> {

    // validate signatures
    let mut multisig = ctx.accounts.multisig.load_mut()?;

    let signatures_left = multisig.sign_multisig(
        &ctx.accounts.signer,
        &Multisig::get_account_infos(&ctx)[1..],
        &Multisig::get_instruction_data(crate::state::AdminInstruction::AddCustody, params)?,
    )?;
    if signatures_left > 0 {
        msg!(
            "Instruction has been signed but more signatures are required: {}",
            signatures_left
        );
        return Ok(signatures_left);
    }

    let pool =&mut ctx.accounts.pool;
    require_keys_eq!(*pool.custodies.last().unwrap(), ctx.accounts.custody.key());

    // record custody data
    let custody =&mut ctx.accounts.custody;
    custody.mint = ctx.accounts.custody_token_mint.key();
    custody.token_account = ctx.accounts.custody_token_account.key();
    custody.decimals = ctx.accounts.custody_token_mint.decimals;
    custody.oracle = params.oracle;
    
    // record bumps
    custody.bump = ctx.bumps.custody;
    custody.token_account_bump = ctx.bumps.custody_token_account;

    Ok(0)
}

#[derive(Accounts)]
#[instruction(params: AddCustodyParams)]
pub struct AddCustody<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"multisig"],
        bump = multisig.load()?.bump
    )]
    pub multisig: AccountLoader<'info, Multisig>,

    #[account(
        mut,
        seeds = [b"contract"],
        bump = contract.bump,
      )]
    pub contract: Box<Account<'info, Contract>>,

    #[account(
        mut,
        seeds = [b"pool", params.pool_name.as_bytes()],
        bump = pool.bump,
    )]
    pub pool: Box<Account<'info, Pool>>,

    #[account(
        init_if_needed,
        payer = signer,
        space = Custody::LEN,
        seeds = [b"custody",
                 pool.key().as_ref(),
                 custody_token_mint.key().as_ref()],
        bump
    )]
    pub custody: Account<'info, Custody>,

    #[account(
        init_if_needed,
        payer = signer,
        token::mint = custody_token_mint,
        token::authority = transfer_authority, // PDA
        seeds = [b"custody_token_account",
                 pool.key().as_ref(),
                 custody_token_mint.key().as_ref()],
        bump
    )]
    pub custody_token_account: Account<'info, TokenAccount>,

    /// CHECK: empty PDA, authority for token accounts
    #[account(
        seeds = [b"transfer_authority"],
        bump = contract.transfer_authority_bump
    )]
    pub transfer_authority: AccountInfo<'info>,

    pub custody_token_mint: Box<Account<'info, Mint>>,
    token_program: Program<'info, Token>,
    system_program: Program<'info, System>,
}
