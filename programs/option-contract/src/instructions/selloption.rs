use std::ops::Div;

use crate::{
    errors::OptionError,
    state::{Lp, OptionDetail, User},
    utils::{black_scholes, SOL_USD_PYTH_ACCOUNT, USDC_DECIMALS, WSOL_DECIMALS},
};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer as SplTransfer},
};
use pyth_sdk_solana::state::SolanaPriceAccount;

pub fn sell_option(
    ctx: Context<SellOption>,
    amount: u64,    // WSOL/USDC account for options, call option - SOL amount, Put option - USDC amount
    strike: f64,    // Strike price
    period: u64,       // Number of days from option creation to expiration
    expired_time: u64, // when the option is expired : Unix epoch time
    is_call: bool,     // true : call option, false : put option
    pay_sol: bool,     // true : sol, false : usdc
) -> Result<()> {
    let signer = &ctx.accounts.signer;
    let signer_ata_wsol = &mut ctx.accounts.signer_ata_wsol;
    let signer_ata_usdc = &mut ctx.accounts.signer_ata_usdc;
    let lp_ata_wsol = &mut ctx.accounts.lp_ata_wsol;
    let lp_ata_usdc = &mut ctx.accounts.lp_ata_usdc;
    let lp = &mut ctx.accounts.lp;
    let token_program = &ctx.accounts.token_program;
    let option_detail = &mut ctx.accounts.option_detail;
    let user = &mut ctx.accounts.user;
    let option_index = user.option_index + 1;

    let price_account_info = &ctx.accounts.pyth_price_account;
    // Get Price Feed from Pyth network price account.
    let price_feed = SolanaPriceAccount::account_info_to_feed(price_account_info)
        .map_err(|_| ProgramError::InvalidAccountData)?;

    // TODO: Update function on Mainnnet
    let price = price_feed.get_price_unchecked();
    // .get_price_no_older_than(current_timestamp, 60).unwrap();

    let oracle_price = (price.price as f64) * 10f64.powi(price.expo);
    let period_year = (period as f64).div(365.0);
    
    // Calculate Premium in usd using black scholes formula.
    let premium = black_scholes(oracle_price, strike, period_year, is_call);
    
    // Calculate Premium in WSOL 
    let premium_sol = (premium.div(oracle_price) * i32::pow(10, WSOL_DECIMALS) as f64) as u64;
    // Calculate Premium in USDC
    let premium_usdc = (premium * i32::pow(10, USDC_DECIMALS) as f64) as u64;

    if pay_sol {

        // Check if the user's WSOL balance is enough to pay premium
        require_gte!(
            signer_ata_wsol.amount,
            premium_sol,
            OptionError::InvalidSignerBalanceError
        );

        // Send WSOL from User to Liquidity Pool as premium
        token::transfer(
            CpiContext::new(
                token_program.to_account_info(),
                SplTransfer {
                    from: signer_ata_wsol.to_account_info(),
                    to: lp_ata_wsol.to_account_info(),
                    authority: signer.to_account_info(),
                },
            ),
            premium_sol,
        )?;

        // Add premium to liquidity pool 
        lp.sol_amount += premium_sol as u64;
        option_detail.premium = premium_sol;

    } else {

        // Check if the user has enough USDC balance to pay premium
        require_gte!(
            signer_ata_usdc.amount,
            premium_usdc,
            OptionError::InvalidSignerBalanceError
        );
        // Send USDC from User to Liquidity Pool as premium
        token::transfer(
            CpiContext::new(
                token_program.to_account_info(),
                SplTransfer {
                    from: signer_ata_usdc.to_account_info(),
                    to: lp_ata_usdc.to_account_info(),
                    authority: signer.to_account_info(),
                },
            ),
            premium_usdc,
        )?;
        
        // Add premium to liquidity pool 
        lp.usdc_amount += premium_usdc as u64;
        option_detail.premium = premium_usdc;
    }

    // Lock assets for call(covered sol)/ put(secured-cash usdc) option
    if is_call {
        require_gte!(lp.sol_amount, amount, OptionError::InvalidPoolBalanceError);
        lp.locked_sol_amount += amount as u64;
        lp.sol_amount -= amount as u64;
        option_detail.sol_amount = amount;
    } else {
        require_gte!(lp.usdc_amount, amount, OptionError::InvalidPoolBalanceError);
        lp.locked_usdc_amount += amount as u64;
        lp.usdc_amount -= amount as u64;
        option_detail.usdc_amount = amount;
    }

    // store option data
    option_detail.index = option_index;
    option_detail.period = period;
    option_detail.expired_date = expired_time as u64;
    option_detail.strike_price = strike;
    option_detail.premium_unit = pay_sol;
    option_detail.option_type = is_call;
    option_detail.valid = true;
    user.option_index = option_index;

    Ok(())
}

#[derive(Accounts)]
pub struct SellOption<'info> {
    #[account(mut)]
    pub signer: Signer<'info>,

    pub wsol_mint: Account<'info, Mint>,
    pub usdc_mint: Account<'info, Mint>,

    #[account(
    mut,
    associated_token::mint = wsol_mint,
    associated_token::authority = signer,
  )]
    pub signer_ata_wsol: Box<Account<'info, TokenAccount>>,

    #[account(
      mut,
      associated_token::mint = usdc_mint,
      associated_token::authority = signer,
    )]
    pub signer_ata_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
    seeds = [b"lp"],
    bump=lp.bump,
  )]
    pub lp: Box<Account<'info, Lp>>,

    #[account(
        mut,
    associated_token::mint = wsol_mint,
    associated_token::authority = lp,
  )]
    pub lp_ata_wsol: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
      associated_token::mint = usdc_mint,
      associated_token::authority = lp,
    )]
    pub lp_ata_usdc: Box<Account<'info, TokenAccount>>,

    #[account(
    init_if_needed,
    payer = signer,
    space=User::LEN,
    seeds = [b"user", signer.key().as_ref()],
    bump,
  )]
    pub user: Box<Account<'info, User>>,

    #[account(
      init,
      payer = signer,
      space=OptionDetail::LEN,
      seeds = [b"option", signer.key().as_ref(), (user.option_index+1).to_le_bytes().as_ref()],
        bump
    )]
    pub option_detail: Box<Account<'info, OptionDetail>>,

    /// CHECK:
    #[account(address = SOL_USD_PYTH_ACCOUNT)]
    pub pyth_price_account: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
